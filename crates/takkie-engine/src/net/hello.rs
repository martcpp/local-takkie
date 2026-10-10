//! Keep-alives: Hello with our name every couple of seconds to every known
//! peer, on any channel, and to static peers; Bye when we stop.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use takkie_core::peers::Peer;
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
    /// Sends Hello with `name` straight away and then every `every`, to
    /// every peer in `known` on any channel and to `static_peers`.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(
        sender: Arc<PacketSender>,
        name: &str,
        every: Duration,
        known: Arc<ArcSwap<Vec<Peer>>>,
        static_peers: Vec<SocketAddr>,
    ) -> io::Result<Self> {
        let name = name.trim().to_string();
        let (stop, stopped) = bounded::<()>(0);
        let handle = thread::Builder::new()
            .name("takkie-hello".into())
            .spawn(move || {
                let everyone = |kind, payload: &[u8]| {
                    let known = known.load();
                    let mut done: Vec<SocketAddr> = Vec::with_capacity(known.len());
                    for to in known
                        .iter()
                        .map(|peer| peer.addr)
                        .chain(static_peers.iter().copied())
                    {
                        if !done.contains(&to) {
                            sender.send_to(kind, Flags::NONE, 0, payload, to);
                            done.push(to);
                        }
                    }
                };
                loop {
                    everyone(PacketKind::Hello, name.as_bytes());
                    if stopped.recv_timeout(every) != Err(RecvTimeoutError::Timeout) {
                        break;
                    }
                }
                everyone(PacketKind::Bye, &[]);
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
    use std::sync::atomic::AtomicU8;
    use std::time::Instant;

    use crossbeam_channel::unbounded;
    use takkie_core::peers::{PeerEvent, PeerSource, PeerTable};
    use takkie_core::protocol::Header;
    use takkie_core::{ChannelId, PeerId};

    use super::*;
    use crate::net::peers::{PEER_TIMEOUT, PeerMessage, PeerOutputs, PeerThread, Peers, apply};
    use crate::net::rx::{RxOutputs, RxThread};
    use crate::net::transport::{MemoryNetwork, MemoryTransport, Transport, UdpTransport};

    const ME: PeerId = PeerId::new(1);
    const THEM: PeerId = PeerId::new(2);

    fn sender(network: &MemoryNetwork) -> Arc<PacketSender> {
        Arc::new(PacketSender::new(
            Arc::new(network.bind(0)),
            ME,
            Arc::new(AtomicU8::new(3)),
            Arc::new(ArcSwap::from_pointee(Vec::new())),
        ))
    }

    fn known(peers: &[(SocketAddr, u8)]) -> Arc<ArcSwap<Vec<Peer>>> {
        let peers = peers
            .iter()
            .enumerate()
            .map(|(i, &(addr, channel))| Peer {
                id: PeerId::new(100 + i as u64),
                name: String::new(),
                addr,
                channel: ChannelId::try_from(channel).unwrap(),
                last_seen: Instant::now(),
                source: PeerSource::Packet,
            })
            .collect();
        Arc::new(ArcSwap::from_pointee(peers))
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
        let hello = HelloThread::spawn(
            sender(&network),
            "  Kitchen ",
            Duration::from_millis(30),
            known(&[(them.local_addr(), 3)]),
            Vec::new(),
        )
        .unwrap();
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
        let hello =
            HelloThread::spawn(sender(&network), "x", HELLO_EVERY, known(&[]), Vec::new()).unwrap();
        let start = Instant::now();
        drop(hello);
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn peers_on_other_channels_still_get_hello() {
        let network = MemoryNetwork::new();
        let away = network.bind(0);
        let hello = HelloThread::spawn(
            sender(&network),
            "Kitchen",
            HELLO_EVERY,
            known(&[(away.local_addr(), 7)]),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(next(&away), Some((PacketKind::Hello, b"Kitchen".to_vec())));
        drop(hello);
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
        let hello = HelloThread::spawn(
            sender(&network),
            "Kitchen",
            HELLO_EVERY,
            known(&[(receiver.local_addr(), 3)]),
            Vec::new(),
        )
        .unwrap();

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

    #[test]
    fn known_and_static_peers_each_get_hello_once() {
        let network = MemoryNetwork::new();
        let listed = network.bind(0);
        let unlisted = network.bind(0);
        let statics = vec![listed.local_addr(), unlisted.local_addr()];
        let hello = HelloThread::spawn(
            sender(&network),
            "Kitchen",
            HELLO_EVERY,
            known(&[(listed.local_addr(), 3)]),
            statics,
        )
        .unwrap();
        assert_eq!(
            next(&unlisted),
            Some((PacketKind::Hello, b"Kitchen".to_vec()))
        );
        assert_eq!(
            next(&listed),
            Some((PacketKind::Hello, b"Kitchen".to_vec()))
        );
        assert_eq!(next(&listed), None);
        drop(hello);
        assert_eq!(next(&unlisted), Some((PacketKind::Bye, Vec::new())));
    }

    struct Node {
        addr: SocketAddr,
        channel: Arc<AtomicU8>,
        targets: Arc<ArcSwap<Vec<SocketAddr>>>,
        news: crossbeam_channel::Sender<PeerMessage>,
        events: crossbeam_channel::Receiver<PeerEvent>,
        _threads: (RxThread, PeerThread, HelloThread),
    }

    fn node(id: u64, name: &str, static_peers: Vec<SocketAddr>) -> Node {
        let transport: Arc<dyn Transport> = Arc::new(UdpTransport::bind(0).unwrap());
        let addr = SocketAddr::from(([127, 0, 0, 1], transport.local_addr().port()));
        wire(transport, addr, id, name, static_peers, PEER_TIMEOUT)
    }

    fn wire(
        transport: Arc<dyn Transport>,
        addr: SocketAddr,
        id: u64,
        name: &str,
        static_peers: Vec<SocketAddr>,
        timeout: Duration,
    ) -> Node {
        let channel = Arc::new(AtomicU8::new(3));
        let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let (audio, _) = unbounded();
        let (news, news_in) = unbounded();
        let rx = RxThread::spawn(
            Arc::clone(&transport),
            PeerId::new(id),
            Arc::clone(&channel),
            RxOutputs {
                audio,
                peers: news.clone(),
            },
        )
        .unwrap();
        let (events, events_out) = unbounded();
        let (mixer, _) = unbounded();
        let table = Peers::new(Arc::clone(&channel), Arc::clone(&targets), timeout);
        let known = table.view();
        let peers = PeerThread::spawn(news_in, table, PeerOutputs { events, mixer }).unwrap();
        let sender = Arc::new(PacketSender::new(
            transport,
            PeerId::new(id),
            Arc::clone(&channel),
            Arc::clone(&targets),
        ));
        let every = Duration::from_millis(100);
        let hello = HelloThread::spawn(sender, name, every, known, static_peers).unwrap();
        Node {
            addr,
            channel,
            targets,
            news,
            events: events_out,
            _threads: (rx, peers, hello),
        }
    }

    fn announce(to: &Node, id: u64, from: &Node) {
        to.news
            .send(PeerMessage::Announced {
                sender: PeerId::new(id),
                name: String::new(),
                channel: ChannelId::try_from(3).unwrap(),
                addr: from.addr,
            })
            .unwrap();
    }

    #[test]
    fn peers_found_over_mdns_survive_a_long_channel_switch() {
        let network = MemoryNetwork::new();
        let memory_node = |id, name| {
            let transport = Arc::new(network.bind(0));
            let addr = transport.local_addr();
            wire(
                transport,
                addr,
                id,
                name,
                Vec::new(),
                Duration::from_millis(400),
            )
        };
        let a = memory_node(1, "Kitchen");
        let b = memory_node(2, "Bedroom");
        announce(&a, 2, &b);
        announce(&b, 1, &a);
        let wait = Duration::from_secs(2);
        assert_eq!(
            a.events.recv_timeout(wait).unwrap(),
            PeerEvent::Joined(PeerId::new(2))
        );
        assert_eq!(
            b.events.recv_timeout(wait).unwrap(),
            PeerEvent::Joined(PeerId::new(1))
        );

        b.channel.store(4, std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_millis(1_500));
        let left = |node: &Node| {
            node.events
                .try_iter()
                .any(|event| matches!(event, PeerEvent::Left(_)))
        };
        assert!(!left(&a), "A forgot B while B was away");
        assert!(!left(&b), "B forgot A while away");
        assert!(a.targets.load().is_empty());

        b.channel.store(3, std::sync::atomic::Ordering::Relaxed);
        thread::sleep(Duration::from_millis(1_000));
        assert_eq!(**a.targets.load(), [b.addr]);
        assert_eq!(**b.targets.load(), [a.addr]);
    }

    #[test]
    fn two_nodes_find_each_other_over_udp_with_one_static_peer_and_no_mdns() {
        let b = node(2, "Bedroom", Vec::new());
        let a = node(1, "Kitchen", vec![b.addr]);
        let wait = Duration::from_secs(3);
        assert_eq!(
            b.events.recv_timeout(wait).unwrap(),
            PeerEvent::Joined(PeerId::new(1))
        );
        assert_eq!(
            a.events.recv_timeout(wait).unwrap(),
            PeerEvent::Joined(PeerId::new(2))
        );
        drop(a);
        assert_eq!(
            b.events.recv_timeout(wait).unwrap(),
            PeerEvent::Left(PeerId::new(1))
        );
    }
}
