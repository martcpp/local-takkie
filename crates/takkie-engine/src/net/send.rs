//! The sending side of the network: every packet goes to each peer on our
//! channel, with one sequence counter across all packet kinds.

use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arc_swap::ArcSwap;
use crossbeam_channel::Receiver;
use takkie_core::protocol::{Flags, HEADER_LEN, Header, PacketKind};
use takkie_core::{ChannelId, PeerId, Seq};

use super::transport::Transport;
use crate::audio::tx::TxEvent;

const MAX_DATAGRAM: usize = 1_500;

/// What happened to sent datagrams.
#[derive(Debug, Default)]
pub struct SendCounters {
    /// Datagrams handed to the OS.
    pub sent: AtomicU64,
    /// Sends that failed.
    pub errors: AtomicU64,
    per_peer: Mutex<HashMap<SocketAddr, u64>>,
}

impl SendCounters {
    /// Failed sends to `peer`.
    #[must_use]
    pub fn errors_to(&self, peer: SocketAddr) -> u64 {
        self.per_peer
            .lock()
            .map(|map| map.get(&peer).copied().unwrap_or(0))
            .unwrap_or(0)
    }

    fn failed(&self, peer: SocketAddr) {
        self.errors.fetch_add(1, Relaxed);
        if let Ok(mut map) = self.per_peer.lock() {
            *map.entry(peer).or_default() += 1;
        }
    }
}

/// Builds packets and sends them to the peers on our channel.
pub struct PacketSender {
    transport: Arc<dyn Transport>,
    me: PeerId,
    channel: Arc<AtomicU8>,
    seq: AtomicU32,
    peers: Arc<ArcSwap<Vec<SocketAddr>>>,
    counters: Arc<SendCounters>,
}

impl PacketSender {
    /// A sender for `me`, reading the channel and the peer list each time.
    #[must_use]
    pub fn new(
        transport: Arc<dyn Transport>,
        me: PeerId,
        channel: Arc<AtomicU8>,
        peers: Arc<ArcSwap<Vec<SocketAddr>>>,
    ) -> Self {
        Self {
            transport,
            me,
            channel,
            seq: AtomicU32::new(0),
            peers,
            counters: Arc::default(),
        }
    }

    /// Send counts, shared.
    #[must_use]
    pub fn counters(&self) -> Arc<SendCounters> {
        Arc::clone(&self.counters)
    }

    /// Whether `addr` is on the current send list.
    #[must_use]
    pub fn is_target(&self, addr: SocketAddr) -> bool {
        self.peers.load().contains(&addr)
    }

    /// Sends one packet to every peer on our channel.
    pub fn send(&self, kind: PacketKind, flags: Flags, timestamp: u32, payload: &[u8]) {
        let peers = self.peers.load();
        self.send_each(kind, flags, timestamp, payload, peers.iter().copied());
    }

    /// Sends one packet to a single address.
    pub fn send_to(
        &self,
        kind: PacketKind,
        flags: Flags,
        timestamp: u32,
        payload: &[u8],
        to: SocketAddr,
    ) {
        self.send_each(kind, flags, timestamp, payload, std::iter::once(to));
    }

    fn send_each(
        &self,
        kind: PacketKind,
        flags: Flags,
        timestamp: u32,
        payload: &[u8],
        to: impl Iterator<Item = SocketAddr>,
    ) {
        let Ok(channel) = ChannelId::try_from(self.channel.load(Relaxed)) else {
            return;
        };
        let mut datagram = [0_u8; MAX_DATAGRAM];
        let len = HEADER_LEN + payload.len();
        let Some((header, body)) = datagram
            .get_mut(..len)
            .and_then(|packet| packet.split_first_chunk_mut::<HEADER_LEN>())
        else {
            self.counters.errors.fetch_add(1, Relaxed);
            return;
        };
        Header {
            kind,
            channel,
            flags,
            sender: self.me,
            seq: Seq::new(self.seq.fetch_add(1, Relaxed)),
            timestamp,
        }
        .encode(header);
        body.copy_from_slice(payload);
        for peer in to {
            match self.transport.send_to(&datagram[..len], peer) {
                Ok(()) => {
                    self.counters.sent.fetch_add(1, Relaxed);
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => self.counters.failed(peer),
            }
        }
    }
}

/// The running send thread. Dropping it stops the thread.
pub struct SendThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SendThread {
    /// Sends the tx thread's Opus packets as Audio packets.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(events: Receiver<TxEvent>, sender: Arc<PacketSender>) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-send".into())
            .spawn(move || {
                while !stopping.load(Relaxed) {
                    match events.recv_timeout(Duration::from_millis(100)) {
                        Ok(TxEvent::Packet(packet)) => {
                            let flags = if packet.end {
                                Flags::END_OF_TRANSMISSION
                            } else {
                                Flags::NONE
                            };
                            sender.send(
                                PacketKind::Audio,
                                flags,
                                packet.timestamp,
                                &packet.payload,
                            );
                        }
                        Ok(TxEvent::Error(error)) => log::warn!("{error}"),
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for SendThread {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;

    use super::*;
    use crate::audio::tx::TxPacket;
    use crate::net::transport::{MemoryNetwork, MemoryTransport, RECV_TIMEOUT};

    fn recv(transport: &MemoryTransport) -> Option<Header> {
        let mut buf = [0; 2048];
        let (len, _) = transport.recv_from(&mut buf).ok()??;
        Header::decode(&buf[..len]).ok().map(|(header, _)| header)
    }

    struct Setup {
        sender: PacketSender,
        peers: Arc<ArcSwap<Vec<SocketAddr>>>,
        channel: Arc<AtomicU8>,
        others: Vec<MemoryTransport>,
    }

    fn setup(listeners: usize) -> Setup {
        let network = MemoryNetwork::new();
        let me: Arc<dyn Transport> = Arc::new(network.bind(0));
        let others: Vec<MemoryTransport> = (0..listeners).map(|_| network.bind(0)).collect();
        let peers = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let channel = Arc::new(AtomicU8::new(4));
        let sender =
            PacketSender::new(me, PeerId::new(9), Arc::clone(&channel), Arc::clone(&peers));
        Setup {
            sender,
            peers,
            channel,
            others,
        }
    }

    #[test]
    fn sends_only_to_the_listed_peers() {
        let s = setup(3);
        s.peers.store(Arc::new(vec![
            s.others[0].local_addr(),
            s.others[2].local_addr(),
        ]));
        s.sender.send(PacketKind::Audio, Flags::NONE, 960, b"opus");
        assert!(recv(&s.others[0]).is_some());
        assert!(recv(&s.others[1]).is_none());
        assert!(recv(&s.others[2]).is_some());
        assert_eq!(s.sender.counters().sent.load(Relaxed), 2);
    }

    #[test]
    fn one_sequence_counter_across_kinds_and_the_current_channel() {
        let s = setup(1);
        s.peers.store(Arc::new(vec![s.others[0].local_addr()]));
        s.sender.send(PacketKind::Audio, Flags::NONE, 0, b"a");
        s.sender.send_to(
            PacketKind::Hello,
            Flags::NONE,
            0,
            b"me",
            s.others[0].local_addr(),
        );
        s.channel.store(7, Relaxed);
        s.sender.send(PacketKind::Bye, Flags::NONE, 0, b"");
        let headers: Vec<Header> = (0..3).filter_map(|_| recv(&s.others[0])).collect();
        let seqs: Vec<u32> = headers.iter().map(|h| h.seq.get()).collect();
        assert_eq!(seqs, [0, 1, 2]);
        assert_eq!(headers[0].sender, PeerId::new(9));
        assert_eq!(headers[2].channel.get(), 7);
        assert_eq!(headers[2].kind, PacketKind::Bye);
    }

    struct Failing(SocketAddr);

    impl Transport for Failing {
        fn local_addr(&self) -> SocketAddr {
            SocketAddr::from(([127, 0, 0, 1], 1))
        }

        fn send_to(&self, _: &[u8], to: SocketAddr) -> io::Result<()> {
            if to == self.0 {
                Err(io::Error::new(ErrorKind::PermissionDenied, "blocked"))
            } else {
                Err(io::Error::new(ErrorKind::WouldBlock, "full"))
            }
        }

        fn recv_from(&self, _: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
            std::thread::sleep(RECV_TIMEOUT);
            Ok(None)
        }
    }

    #[test]
    fn errors_are_counted_per_peer_and_would_block_is_ignored() {
        let bad = SocketAddr::from(([10, 0, 0, 1], 5));
        let busy = SocketAddr::from(([10, 0, 0, 2], 5));
        let peers = Arc::new(ArcSwap::from_pointee(vec![bad, busy]));
        let sender = PacketSender::new(
            Arc::new(Failing(bad)),
            PeerId::new(1),
            Arc::new(AtomicU8::new(1)),
            peers,
        );
        sender.send(PacketKind::Audio, Flags::NONE, 0, b"x");
        sender.send(PacketKind::Audio, Flags::NONE, 0, b"x");
        let counters = sender.counters();
        assert_eq!(counters.errors.load(Relaxed), 2);
        assert_eq!(counters.errors_to(bad), 2);
        assert_eq!(counters.errors_to(busy), 0);
        assert_eq!(counters.sent.load(Relaxed), 0);
    }

    #[test]
    fn a_payload_too_big_for_one_datagram_is_an_error_not_a_panic() {
        let s = setup(1);
        s.peers.store(Arc::new(vec![s.others[0].local_addr()]));
        s.sender
            .send(PacketKind::Audio, Flags::NONE, 0, &[0; 4_000]);
        assert_eq!(s.sender.counters().errors.load(Relaxed), 1);
        assert!(recv(&s.others[0]).is_none());
    }

    #[test]
    fn the_thread_sends_tx_packets_as_audio() {
        let s = setup(1);
        s.peers.store(Arc::new(vec![s.others[0].local_addr()]));
        let (events, received) = unbounded();
        let thread = SendThread::spawn(received, Arc::new(s.sender)).unwrap();
        events
            .send(TxEvent::Packet(TxPacket {
                payload: b"opus".to_vec(),
                timestamp: 1_920,
                end: true,
            }))
            .unwrap();
        let header = recv(&s.others[0]).unwrap();
        drop(thread);
        assert_eq!(header.kind, PacketKind::Audio);
        assert_eq!(header.timestamp, 1_920);
        assert!(header.flags.contains(Flags::END_OF_TRANSMISSION));
    }
}
