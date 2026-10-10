//! The receiving thread: checks each datagram and routes it, our channel's
//! audio to the mixer and Hello and Bye to the peer table. On a private
//! channel only packets that open with the channel key are trusted.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::Sender;
use takkie_core::key::ChannelKey;
use takkie_core::protocol::{Flags, Header, PacketKind};
use takkie_core::replay::Replays;
use takkie_core::seal::open_packet;
use takkie_core::{ChannelId, PeerId};

use super::peers::PeerMessage;
use super::transport::Transport;
use crate::audio::mix::{MixInput, RxPacket};
use crate::threads::{STOP_WITHIN, join_within};

const MAX_NAME: usize = 64;
const HEARD_EVERY: Duration = Duration::from_secs(1);
// How long a sender who sealed a packet counts as private.
const PRIVATE_FOR: Duration = Duration::from_secs(30);
const PRIVATE_LIMIT: usize = 1024;
// A sender stays on the mismatch list this long after their last bad packet.
const MISMATCH_FOR: Duration = Duration::from_secs(5);
const MISMATCH_LIMIT: usize = 64;

/// Senders on our channel whose packets don't match our key, recently.
#[derive(Debug, Default)]
pub struct Mismatches {
    seen: HashMap<PeerId, Instant>,
}

impl Mismatches {
    /// Notes a bad packet from `sender`; true if the list changed.
    pub fn note(&mut self, sender: PeerId, now: Instant) -> bool {
        // Sender ids in packets we can't verify are unverified, so cap it.
        if self.seen.len() >= MISMATCH_LIMIT && !self.seen.contains_key(&sender) {
            return false;
        }
        self.seen.insert(sender, now).is_none()
    }

    /// Drops senders quiet for a while; true if the list changed.
    pub fn prune(&mut self, now: Instant) -> bool {
        let before = self.seen.len();
        self.seen
            .retain(|_, at| now.saturating_duration_since(*at) < MISMATCH_FOR);
        self.seen.len() != before
    }

    /// The senders, in id order.
    #[must_use]
    pub fn list(&self) -> Vec<PeerId> {
        let mut senders: Vec<PeerId> = self.seen.keys().copied().collect();
        senders.sort();
        senders
    }
}

/// What happened to received datagrams.
#[derive(Debug, Default)]
pub struct RxCounters {
    /// Datagrams received.
    pub received: AtomicU64,
    /// Not a valid takkie header.
    pub invalid: AtomicU64,
    /// For another channel, and not presence we can use.
    pub other_channel: AtomicU64,
    /// Our own, looped back.
    pub own: AtomicU64,
    /// On our channel but not matching our key, or a Bye we can't trust.
    pub mismatched: AtomicU64,
    /// Sealed packets seen before.
    pub replayed: AtomicU64,
}

/// Where one datagram goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Not a valid header.
    Invalid,
    /// Sent by us.
    Own,
    /// Another channel's audio, or a sealed Bye from there.
    OtherChannel,
    /// Audio for the mixer.
    Audio(RxPacket),
    /// News for the peer table.
    Peer(PeerMessage),
    /// A Bye in the clear for a sender who seals their packets.
    Untrusted,
    /// On our channel, but it doesn't match our key. A Hello still tells us
    /// the sender exists.
    Mismatch {
        /// Who sent it.
        sender: PeerId,
        /// Presence to pass on, if it was a Hello.
        presence: Option<PeerMessage>,
    },
    /// A sealed packet we already accepted.
    Replayed,
}

/// Decides where datagrams go, remembering what's needed to do it safely.
#[derive(Debug)]
pub struct Router {
    me: PeerId,
    replays: Replays,
    private: HashMap<PeerId, Instant>,
}

impl Router {
    /// A router for the engine with id `me`.
    #[must_use]
    pub fn new(me: PeerId) -> Self {
        Self {
            me,
            replays: Replays::new(),
            private: HashMap::new(),
        }
    }

    /// Routes one datagram that arrived at `now`, decrypting it in place
    /// when it is sealed for our channel and `key` opens it.
    pub fn route(
        &mut self,
        datagram: &mut [u8],
        from: SocketAddr,
        channel: ChannelId,
        key: Option<&ChannelKey>,
        now: Instant,
    ) -> Route {
        let Ok((header, _)) = Header::decode(datagram) else {
            return Route::Invalid;
        };
        if header.sender == self.me {
            return Route::Own;
        }
        let sender = header.sender;
        let sealed = header.flags.contains(Flags::ENCRYPTED);
        if sealed {
            self.saw_sealed(sender, now);
        }
        let presence = PeerMessage::Heard {
            sender,
            channel: header.channel,
            addr: from,
        };
        let hello = header.kind == PacketKind::Hello;

        if header.channel != channel {
            return match (header.kind, sealed) {
                (PacketKind::Hello, true) => Route::Peer(presence),
                (PacketKind::Audio, _) | (PacketKind::Bye, true) => Route::OtherChannel,
                (PacketKind::Hello | PacketKind::Bye, false) => self.plain(datagram, from, now),
            };
        }
        match (sealed, key) {
            (false, None) => self.plain(datagram, from, now),
            (true, Some(key)) => match open_packet(key, datagram) {
                Ok((header, payload)) if self.replays.accept(sender, header.seq) => {
                    if header.kind == PacketKind::Bye {
                        self.replays.forget(sender);
                        self.private.remove(&sender);
                    }
                    deliver(&header, payload, from)
                }
                Ok(_) => Route::Replayed,
                Err(_) => Route::Mismatch {
                    sender,
                    presence: hello.then_some(presence),
                },
            },
            (true, None) | (false, Some(_)) => Route::Mismatch {
                sender,
                presence: hello.then_some(presence),
            },
        }
    }

    // A packet nobody vouched for. Fine for open peers, but it must not let
    // an outsider say Bye for someone who seals their packets.
    fn plain(&self, datagram: &[u8], from: SocketAddr, now: Instant) -> Route {
        let Ok((header, payload)) = Header::decode(datagram) else {
            return Route::Invalid;
        };
        if header.kind == PacketKind::Bye && self.is_private(header.sender, now) {
            return Route::Untrusted;
        }
        deliver(&header, payload, from)
    }

    fn is_private(&self, sender: PeerId, now: Instant) -> bool {
        self.private
            .get(&sender)
            .is_some_and(|at| now.saturating_duration_since(*at) < PRIVATE_FOR)
    }

    fn saw_sealed(&mut self, sender: PeerId, now: Instant) {
        // Ids in sealed packets we can't open are unverified, so the map is capped.
        if self.private.len() >= PRIVATE_LIMIT {
            self.private
                .retain(|_, at| now.saturating_duration_since(*at) < PRIVATE_FOR);
        }
        if self.private.len() < PRIVATE_LIMIT {
            self.private.insert(sender, now);
        }
    }
}

fn deliver(header: &Header, payload: &[u8], from: SocketAddr) -> Route {
    match header.kind {
        PacketKind::Audio => Route::Audio(RxPacket {
            sender: header.sender,
            seq: header.seq,
            payload: payload.to_vec(),
            end: header.flags.contains(Flags::END_OF_TRANSMISSION),
        }),
        PacketKind::Hello => Route::Peer(PeerMessage::Hello {
            sender: header.sender,
            name: name(payload),
            channel: header.channel,
            addr: from,
        }),
        PacketKind::Bye => Route::Peer(PeerMessage::Bye {
            sender: header.sender,
        }),
    }
}

fn name(payload: &[u8]) -> String {
    String::from_utf8_lossy(payload)
        .trim()
        .chars()
        .take(MAX_NAME)
        .collect()
}

/// Where routed packets go.
pub struct RxOutputs {
    /// Audio for the mix thread.
    pub audio: Sender<MixInput>,
    /// Peer news for the control thread.
    pub peers: Sender<PeerMessage>,
}

/// The running rx thread. Dropping it stops the thread.
pub struct RxThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    counters: Arc<RxCounters>,
    mismatched: Arc<ArcSwap<Vec<PeerId>>>,
}

impl RxThread {
    /// Starts receiving on `transport` as `me` on an open channel, keeping
    /// packets for the channel currently in `channel`.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(
        transport: Arc<dyn Transport>,
        me: PeerId,
        channel: Arc<AtomicU8>,
        outputs: RxOutputs,
    ) -> io::Result<Self> {
        Self::spawn_keyed(transport, me, channel, Arc::default(), outputs)
    }

    /// Like [`spawn`](Self::spawn), using whatever key is in `key` at the
    /// time each packet arrives.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn_keyed(
        transport: Arc<dyn Transport>,
        me: PeerId,
        channel: Arc<AtomicU8>,
        key: Arc<ArcSwapOption<ChannelKey>>,
        outputs: RxOutputs,
    ) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let counters = Arc::new(RxCounters::default());
        let counting = Arc::clone(&counters);
        let mismatched: Arc<ArcSwap<Vec<PeerId>>> = Arc::default();
        let publishing = Arc::clone(&mismatched);
        let handle = thread::Builder::new()
            .name("takkie-rx".into())
            .spawn(move || {
                let mut buf = [0_u8; 2048];
                let mut router = Router::new(me);
                let mut mismatches = Mismatches::default();
                let mut last_heard: HashMap<PeerId, Instant> = HashMap::new();
                while !stopping.load(Relaxed) {
                    if mismatches.prune(Instant::now()) {
                        publishing.store(Arc::new(mismatches.list()));
                    }
                    let (len, from) = match transport.recv_from(&mut buf) {
                        Ok(Some(received)) => received,
                        Ok(None) => continue,
                        Err(error) => {
                            tracing::warn!("receive failed: {error}");
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    };
                    counting.received.fetch_add(1, Relaxed);
                    let Ok(current) = ChannelId::try_from(channel.load(Relaxed)) else {
                        continue;
                    };
                    let Some(datagram) = buf.get_mut(..len) else {
                        continue;
                    };
                    let now = Instant::now();
                    let key = key.load();
                    let sent = match router.route(datagram, from, current, key.as_deref(), now) {
                        Route::Invalid => {
                            counting.invalid.fetch_add(1, Relaxed);
                            true
                        }
                        Route::Own => {
                            counting.own.fetch_add(1, Relaxed);
                            true
                        }
                        Route::OtherChannel => {
                            counting.other_channel.fetch_add(1, Relaxed);
                            true
                        }
                        Route::Replayed => {
                            counting.replayed.fetch_add(1, Relaxed);
                            true
                        }
                        Route::Untrusted => {
                            counting.mismatched.fetch_add(1, Relaxed);
                            true
                        }
                        Route::Mismatch { sender, presence } => {
                            counting.mismatched.fetch_add(1, Relaxed);
                            if mismatches.note(sender, now) {
                                publishing.store(Arc::new(mismatches.list()));
                            }
                            presence.is_none_or(|seen| outputs.peers.send(seen).is_ok())
                        }
                        Route::Audio(packet) => {
                            let due = last_heard
                                .get(&packet.sender)
                                .is_none_or(|at| now.duration_since(*at) >= HEARD_EVERY);
                            if due {
                                last_heard.insert(packet.sender, now);
                                let _ = outputs.peers.send(PeerMessage::Heard {
                                    sender: packet.sender,
                                    channel: current,
                                    addr: from,
                                });
                            }
                            outputs.audio.send(MixInput::Packet(packet)).is_ok()
                        }
                        Route::Peer(message) => {
                            if let PeerMessage::Bye { sender } = &message {
                                last_heard.remove(sender);
                            }
                            outputs.peers.send(message).is_ok()
                        }
                    };
                    if !sent {
                        return;
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
            counters,
            mismatched,
        })
    }

    /// Senders on our channel whose packets didn't match our key in the
    /// last few seconds, republished when it changes.
    #[must_use]
    pub fn mismatched(&self) -> Arc<ArcSwap<Vec<PeerId>>> {
        Arc::clone(&self.mismatched)
    }

    /// What happened to received datagrams.
    #[must_use]
    pub fn counters(&self) -> Arc<RxCounters> {
        Arc::clone(&self.counters)
    }
}

impl Drop for RxThread {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(handle) = self.handle.take() {
            join_within(handle, STOP_WITHIN);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;
    use takkie_core::Seq;
    use takkie_core::protocol::HEADER_LEN;
    use takkie_core::seal::{TAG_LEN, seal_packet};

    use super::*;
    use crate::net::transport::MemoryNetwork;

    const ME: PeerId = PeerId::new(1);
    const THEM: PeerId = PeerId::new(2);

    fn channel(n: u8) -> ChannelId {
        ChannelId::try_from(n).unwrap()
    }

    fn header(kind: PacketKind, sender: u64, ch: u8, flags: Flags, seq: u32) -> Header {
        Header {
            kind,
            channel: channel(ch),
            flags,
            sender: PeerId::new(sender),
            seq: Seq::new(seq),
            timestamp: 960,
        }
    }

    fn datagram(kind: PacketKind, sender: u64, ch: u8, flags: Flags, payload: &[u8]) -> Vec<u8> {
        let mut bytes = [0; HEADER_LEN];
        header(kind, sender, ch, flags, 7).encode(&mut bytes);
        [&bytes[..], payload].concat()
    }

    fn sealed(kind: PacketKind, ch: u8, seq: u32, payload: &[u8], key: &ChannelKey) -> Vec<u8> {
        let mut out = vec![0; HEADER_LEN + payload.len() + TAG_LEN];
        seal_packet(
            key,
            header(kind, 2, ch, Flags::NONE, seq),
            payload,
            &mut out,
        )
        .unwrap();
        out
    }

    fn key(byte: u8) -> ChannelKey {
        ChannelKey::from_bytes([byte; 32])
    }

    fn from() -> SocketAddr {
        SocketAddr::from(([192, 168, 1, 20], 41_000))
    }

    fn route(bytes: &[u8]) -> Route {
        Router::new(ME).route(
            &mut bytes.to_vec(),
            from(),
            channel(3),
            None,
            Instant::now(),
        )
    }

    fn presence(ch: u8) -> PeerMessage {
        PeerMessage::Heard {
            sender: THEM,
            channel: channel(ch),
            addr: from(),
        }
    }

    #[test]
    fn audio_on_our_channel_goes_to_the_mixer() {
        let bytes = datagram(PacketKind::Audio, 2, 3, Flags::END_OF_TRANSMISSION, b"opus");
        assert_eq!(
            route(&bytes),
            Route::Audio(RxPacket {
                sender: THEM,
                seq: Seq::new(7),
                payload: b"opus".to_vec(),
                end: true,
            })
        );
    }

    #[test]
    fn hello_and_bye_go_to_the_peer_table() {
        let hello = datagram(PacketKind::Hello, 2, 3, Flags::NONE, b"  Kitchen  ");
        assert_eq!(
            route(&hello),
            Route::Peer(PeerMessage::Hello {
                sender: THEM,
                name: "Kitchen".into(),
                channel: channel(3),
                addr: from(),
            })
        );
        let bye = datagram(PacketKind::Bye, 2, 3, Flags::NONE, b"");
        assert_eq!(route(&bye), Route::Peer(PeerMessage::Bye { sender: THEM }));
    }

    #[test]
    fn names_are_cleaned_and_capped() {
        let long = "a".repeat(200);
        let hello = datagram(PacketKind::Hello, 2, 3, Flags::NONE, long.as_bytes());
        let Route::Peer(PeerMessage::Hello { name, .. }) = route(&hello) else {
            unreachable!("a hello routes to the peer table");
        };
        assert_eq!(name.chars().count(), MAX_NAME);
        let bad = datagram(PacketKind::Hello, 2, 3, Flags::NONE, &[0x4B, 0xFF, 0x4B]);
        let Route::Peer(PeerMessage::Hello { name, .. }) = route(&bad) else {
            unreachable!("a hello routes to the peer table");
        };
        assert_eq!(name, "K\u{FFFD}K");
    }

    #[test]
    fn other_channels_our_own_and_junk_are_dropped() {
        let other = datagram(PacketKind::Audio, 2, 4, Flags::NONE, b"x");
        assert_eq!(route(&other), Route::OtherChannel);
        let away = datagram(PacketKind::Hello, 2, 4, Flags::NONE, b"Away");
        assert_eq!(
            route(&away),
            Route::Peer(PeerMessage::Hello {
                sender: THEM,
                name: "Away".into(),
                channel: channel(4),
                addr: from(),
            })
        );
        let own = datagram(PacketKind::Audio, 1, 3, Flags::NONE, b"x");
        assert_eq!(route(&own), Route::Own);
        assert_eq!(route(b"not takkie"), Route::Invalid);
    }

    #[test]
    fn sealed_packets_open_with_our_key_and_only_once() {
        let mut router = Router::new(ME);
        let now = Instant::now();
        let audio = sealed(PacketKind::Audio, 3, 10, b"opus", &key(7));
        let mut go = |bytes: &[u8]| {
            router.route(&mut bytes.to_vec(), from(), channel(3), Some(&key(7)), now)
        };
        assert_eq!(
            go(&audio),
            Route::Audio(RxPacket {
                sender: THEM,
                seq: Seq::new(10),
                payload: b"opus".to_vec(),
                end: false,
            })
        );
        assert_eq!(go(&audio), Route::Replayed);
        let hello = sealed(PacketKind::Hello, 3, 11, b"Kitchen", &key(7));
        assert_eq!(
            go(&hello),
            Route::Peer(PeerMessage::Hello {
                sender: THEM,
                name: "Kitchen".into(),
                channel: channel(3),
                addr: from(),
            })
        );
    }

    #[test]
    fn the_wrong_key_or_a_missing_one_is_a_mismatch() {
        let now = Instant::now();
        let audio = sealed(PacketKind::Audio, 3, 10, b"opus", &key(7));
        let hello = sealed(PacketKind::Hello, 3, 11, b"Kitchen", &key(7));
        let clear = datagram(PacketKind::Audio, 2, 3, Flags::NONE, b"opus");
        let quiet = Route::Mismatch {
            sender: THEM,
            presence: None,
        };
        let seen = Route::Mismatch {
            sender: THEM,
            presence: Some(presence(3)),
        };
        let go = |bytes: &[u8], ours: Option<&ChannelKey>| {
            Router::new(ME).route(&mut bytes.to_vec(), from(), channel(3), ours, now)
        };
        assert_eq!(go(&audio, Some(&key(8))), quiet);
        assert_eq!(go(&hello, Some(&key(8))), seen);
        assert_eq!(go(&audio, None), quiet);
        assert_eq!(go(&hello, None), seen);
        assert_eq!(go(&clear, Some(&key(7))), quiet);
    }

    #[test]
    fn a_tampered_packet_does_not_poison_the_replay_window() {
        let mut router = Router::new(ME);
        let now = Instant::now();
        let good = sealed(PacketKind::Audio, 3, 10, b"opus", &key(7));
        let mut forged = sealed(PacketKind::Audio, 3, 500, b"opus", &key(9));
        let mut go = |bytes: &mut [u8]| router.route(bytes, from(), channel(3), Some(&key(7)), now);
        assert!(matches!(go(&mut forged), Route::Mismatch { .. }));
        assert!(matches!(go(&mut good.clone()), Route::Audio(_)));
    }

    #[test]
    fn sealed_presence_from_another_channel_is_shown_but_not_trusted() {
        let mut router = Router::new(ME);
        let now = Instant::now();
        let mut go = |bytes: &[u8]| {
            router.route(&mut bytes.to_vec(), from(), channel(3), Some(&key(7)), now)
        };
        let hello = sealed(PacketKind::Hello, 5, 10, b"Kitchen", &key(9));
        assert_eq!(go(&hello), Route::Peer(presence(5)));
        let bye = sealed(PacketKind::Bye, 5, 11, b"", &key(9));
        assert_eq!(go(&bye), Route::OtherChannel);
        let audio = sealed(PacketKind::Audio, 5, 12, b"opus", &key(9));
        assert_eq!(go(&audio), Route::OtherChannel);
    }

    #[test]
    fn an_outsider_cannot_say_bye_for_a_private_peer() {
        let mut router = Router::new(ME);
        let now = Instant::now();
        let forged_bye = datagram(PacketKind::Bye, 2, 5, Flags::NONE, b"");
        let mut go = |bytes: &[u8], at: Instant| {
            router.route(&mut bytes.to_vec(), from(), channel(3), None, at)
        };
        assert_eq!(
            go(&forged_bye, now),
            Route::Peer(PeerMessage::Bye { sender: THEM })
        );
        go(&sealed(PacketKind::Hello, 5, 10, b"Kitchen", &key(9)), now);
        assert_eq!(go(&forged_bye, now), Route::Untrusted);
        assert_eq!(
            go(&forged_bye, now + PRIVATE_FOR),
            Route::Peer(PeerMessage::Bye { sender: THEM })
        );
    }

    #[test]
    fn mismatches_are_listed_once_and_fade() {
        let mut mismatches = Mismatches::default();
        let start = Instant::now();
        assert!(mismatches.note(THEM, start));
        assert!(!mismatches.note(THEM, start + Duration::from_secs(1)));
        assert!(mismatches.note(PeerId::new(9), start));
        assert_eq!(mismatches.list(), [THEM, PeerId::new(9)]);
        assert!(!mismatches.prune(start + Duration::from_secs(4)));
        assert!(mismatches.prune(start + Duration::from_secs(5)));
        assert_eq!(mismatches.list(), [THEM]);
        assert!(mismatches.prune(start + Duration::from_secs(6)));
        assert!(mismatches.list().is_empty());
    }

    #[test]
    fn the_mismatch_list_is_capped() {
        let mut mismatches = Mismatches::default();
        let now = Instant::now();
        for id in 0..200 {
            mismatches.note(PeerId::new(id), now);
        }
        assert_eq!(mismatches.list().len(), MISMATCH_LIMIT);
    }

    #[test]
    fn the_thread_routes_and_counts() {
        let network = MemoryNetwork::new();
        let rx = Arc::new(network.bind(0));
        let peer = network.bind(0);
        let (audio, audio_out) = unbounded();
        let (peers, peers_out) = unbounded();
        let thread = RxThread::spawn(
            Arc::clone(&rx) as Arc<dyn Transport>,
            ME,
            Arc::new(AtomicU8::new(3)),
            RxOutputs { audio, peers },
        )
        .unwrap();

        let to = rx.local_addr();
        peer.send_to(&datagram(PacketKind::Audio, 2, 3, Flags::NONE, b"a"), to)
            .unwrap();
        peer.send_to(&datagram(PacketKind::Audio, 2, 3, Flags::NONE, b"b"), to)
            .unwrap();
        peer.send_to(&datagram(PacketKind::Audio, 2, 9, Flags::NONE, b"c"), to)
            .unwrap();
        peer.send_to(b"junk", to).unwrap();
        peer.send_to(&sealed(PacketKind::Audio, 3, 1, b"d", &key(7)), to)
            .unwrap();
        peer.send_to(&datagram(PacketKind::Bye, 4, 3, Flags::NONE, b""), to)
            .unwrap();

        let wait = Duration::from_secs(2);
        let payload = || match audio_out.recv_timeout(wait).unwrap() {
            MixInput::Packet(packet) => packet.payload,
            MixInput::Left(_) => Vec::new(),
        };
        assert_eq!(payload(), b"a");
        assert_eq!(payload(), b"b");
        assert!(matches!(
            peers_out.recv_timeout(wait).unwrap(),
            PeerMessage::Heard { addr, .. } if addr == peer.local_addr()
        ));
        assert_eq!(
            peers_out.recv_timeout(wait).unwrap(),
            PeerMessage::Bye {
                sender: PeerId::new(4)
            }
        );
        let counters = thread.counters();
        let mismatched = thread.mismatched();
        drop(thread);
        assert_eq!(counters.received.load(Relaxed), 6);
        assert_eq!(counters.other_channel.load(Relaxed), 1);
        assert_eq!(counters.invalid.load(Relaxed), 1);
        assert_eq!(counters.mismatched.load(Relaxed), 1);
        assert_eq!(**mismatched.load(), [THEM]);
    }
}
