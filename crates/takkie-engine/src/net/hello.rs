//! Keep-alives: a Hello with our name to the peers on our channel every
//! couple of seconds, and a Bye when we stop.

use std::io;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use takkie_core::protocol::{Flags, PacketKind};

use super::send::PacketSender;

/// How often Hello goes out.
pub const HELLO_EVERY: Duration = Duration::from_secs(2);

/// The running Hello thread. Dropping it sends Bye and stops the thread.
pub struct HelloThread {
    stop: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl HelloThread {
    /// Sends Hello with `name` straight away and then every `every`.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(sender: Arc<PacketSender>, name: &str, every: Duration) -> io::Result<Self> {
        let name = name.trim().to_string();
        let (stop, stopped) = bounded::<()>(0);
        let handle = thread::Builder::new()
            .name("takkie-hello".into())
            .spawn(move || {
                loop {
                    sender.send(PacketKind::Hello, Flags::NONE, 0, name.as_bytes());
                    if stopped.recv_timeout(every) != Err(RecvTimeoutError::Timeout) {
                        break;
                    }
                }
                sender.send(PacketKind::Bye, Flags::NONE, 0, &[]);
            })?;
        Ok(Self {
            stop: Some(stop),
            handle: Some(handle),
        })
    }
}

impl Drop for HelloThread {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicU8;
    use std::time::Instant;

    use arc_swap::ArcSwap;
    use crossbeam_channel::unbounded;
    use takkie_core::PeerId;
    use takkie_core::peers::{PeerEvent, PeerTable};
    use takkie_core::protocol::Header;

    use super::*;
    use crate::net::peers::apply;
    use crate::net::rx::{RxOutputs, RxThread};
    use crate::net::transport::{MemoryNetwork, MemoryTransport, Transport};

    const ME: PeerId = PeerId::new(1);
    const THEM: PeerId = PeerId::new(2);

    fn sender_to(network: &MemoryNetwork, peers: Vec<SocketAddr>) -> Arc<PacketSender> {
        Arc::new(PacketSender::new(
            Arc::new(network.bind(0)),
            ME,
            Arc::new(AtomicU8::new(3)),
            Arc::new(ArcSwap::from_pointee(peers)),
        ))
    }

    fn next(transport: &MemoryTransport) -> Option<(PacketKind, Vec<u8>)> {
        let mut buf = [0; 2048];
        let (len, _) = transport.recv_from(&mut buf).ok()??;
        let (header, payload) = Header::decode(&buf[..len]).ok()?;
        Some((header.kind, payload.to_vec()))
    }

    #[test]
    fn hello_repeats_with_the_name_and_bye_comes_last() {
        let network = MemoryNetwork::new();
        let them = network.bind(0);
        let sender = sender_to(&network, vec![them.local_addr()]);
        let hello = HelloThread::spawn(sender, "  Kitchen ", Duration::from_millis(30)).unwrap();
        for _ in 0..3 {
            assert_eq!(next(&them), Some((PacketKind::Hello, b"Kitchen".to_vec())));
        }
        drop(hello);
        let mut last = None;
        while let Some(packet) = next(&them) {
            last = Some(packet);
        }
        assert_eq!(last, Some((PacketKind::Bye, Vec::new())));
    }

    #[test]
    fn stopping_does_not_wait_for_the_next_hello() {
        let network = MemoryNetwork::new();
        let hello = HelloThread::spawn(sender_to(&network, Vec::new()), "x", HELLO_EVERY).unwrap();
        let start = Instant::now();
        drop(hello);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn peers_learn_our_name_and_see_us_leave() {
        let network = MemoryNetwork::new();
        let receiver: Arc<dyn Transport> = Arc::new(network.bind(0));
        let (audio, _audio_out) = unbounded();
        let (peers, news) = unbounded();
        let _rx = RxThread::spawn(
            Arc::clone(&receiver),
            THEM,
            Arc::new(AtomicU8::new(3)),
            RxOutputs { audio, peers },
        )
        .unwrap();
        let sender = sender_to(&network, vec![receiver.local_addr()]);
        let hello = HelloThread::spawn(sender, "Kitchen", HELLO_EVERY).unwrap();

        let mut table = PeerTable::new();
        let wait = Duration::from_secs(2);
        let joined = apply(&mut table, news.recv_timeout(wait).unwrap(), Instant::now());
        assert_eq!(joined, Some(PeerEvent::Joined(ME)));
        assert_eq!(table.get(ME).unwrap().name, "Kitchen");

        drop(hello);
        let left = apply(&mut table, news.recv_timeout(wait).unwrap(), Instant::now());
        assert_eq!(left, Some(PeerEvent::Left(ME)));
        assert!(table.is_empty());
    }
}
