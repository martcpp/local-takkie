//! The receiving thread: decodes each datagram's header and routes it, our
//! channel's audio to the mixer and every channel's Hello and Bye to the
//! peer table.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use takkie_core::protocol::{Flags, Header, PacketKind};
use takkie_core::{ChannelId, PeerId};

use super::peers::PeerMessage;
use super::transport::Transport;
use crate::audio::mix::{MixInput, RxPacket};
use crate::threads::{STOP_WITHIN, join_within};

const MAX_NAME: usize = 64;
const HEARD_EVERY: Duration = Duration::from_secs(1);

/// What happened to received datagrams.
#[derive(Debug, Default)]
pub struct RxCounters {
    /// Datagrams received.
    pub received: AtomicU64,
    /// Not a valid takkie header.
    pub invalid: AtomicU64,
    /// Audio for another channel.
    pub other_channel: AtomicU64,
    /// Our own, looped back.
    pub own: AtomicU64,
}

/// Where one datagram goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Not a valid header.
    Invalid,
    /// Sent by us.
    Own,
    /// Audio from someone on another channel.
    OtherChannel,
    /// Audio for the mixer.
    Audio(RxPacket),
    /// News for the peer table.
    Peer(PeerMessage),
}

/// Decides where a datagram goes.
#[must_use]
pub fn route(datagram: &[u8], from: SocketAddr, me: PeerId, channel: ChannelId) -> Route {
    let Ok((header, payload)) = Header::decode(datagram) else {
        return Route::Invalid;
    };
    if header.sender == me {
        return Route::Own;
    }
    match header.kind {
        PacketKind::Audio if header.channel != channel => Route::OtherChannel,
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
}

impl RxThread {
    /// Starts receiving on `transport` as `me`, keeping packets for the
    /// channel currently in `channel`.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(
        transport: Arc<dyn Transport>,
        me: PeerId,
        channel: Arc<AtomicU8>,
        outputs: RxOutputs,
    ) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let counters = Arc::new(RxCounters::default());
        let counting = Arc::clone(&counters);
        let handle = thread::Builder::new()
            .name("takkie-rx".into())
            .spawn(move || {
                let mut buf = [0_u8; 2048];
                let mut last_heard: HashMap<PeerId, Instant> = HashMap::new();
                while !stopping.load(Relaxed) {
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
                    let Some(datagram) = buf.get(..len) else {
                        continue;
                    };
                    let sent = match route(datagram, from, me, current) {
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
                        Route::Audio(packet) => {
                            let now = Instant::now();
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
        })
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

    use super::*;
    use crate::net::transport::MemoryNetwork;

    fn channel(n: u8) -> ChannelId {
        ChannelId::try_from(n).unwrap()
    }

    fn datagram(kind: PacketKind, sender: u64, ch: u8, flags: Flags, payload: &[u8]) -> Vec<u8> {
        let header = Header {
            kind,
            channel: channel(ch),
            flags,
            sender: PeerId::new(sender),
            seq: Seq::new(7),
            timestamp: 960,
        };
        let mut bytes = [0; HEADER_LEN];
        header.encode(&mut bytes);
        [&bytes[..], payload].concat()
    }

    fn from() -> SocketAddr {
        SocketAddr::from(([192, 168, 1, 20], 41_000))
    }

    const ME: PeerId = PeerId::new(1);

    #[test]
    fn audio_on_our_channel_goes_to_the_mixer() {
        let bytes = datagram(PacketKind::Audio, 2, 3, Flags::END_OF_TRANSMISSION, b"opus");
        assert_eq!(
            route(&bytes, from(), ME, channel(3)),
            Route::Audio(RxPacket {
                sender: PeerId::new(2),
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
            route(&hello, from(), ME, channel(3)),
            Route::Peer(PeerMessage::Hello {
                sender: PeerId::new(2),
                name: "Kitchen".into(),
                channel: channel(3),
                addr: from(),
            })
        );
        let bye = datagram(PacketKind::Bye, 2, 3, Flags::NONE, b"");
        assert_eq!(
            route(&bye, from(), ME, channel(3)),
            Route::Peer(PeerMessage::Bye {
                sender: PeerId::new(2)
            })
        );
    }

    #[test]
    fn names_are_cleaned_and_capped() {
        let long = "a".repeat(200);
        let hello = datagram(PacketKind::Hello, 2, 3, Flags::NONE, long.as_bytes());
        let Route::Peer(PeerMessage::Hello { name, .. }) = route(&hello, from(), ME, channel(3))
        else {
            unreachable!("a hello routes to the peer table");
        };
        assert_eq!(name.chars().count(), MAX_NAME);
        let bad = datagram(PacketKind::Hello, 2, 3, Flags::NONE, &[0x4B, 0xFF, 0x4B]);
        let Route::Peer(PeerMessage::Hello { name, .. }) = route(&bad, from(), ME, channel(3))
        else {
            unreachable!("a hello routes to the peer table");
        };
        assert_eq!(name, "K\u{FFFD}K");
    }

    #[test]
    fn other_channels_our_own_and_junk_are_dropped() {
        let other = datagram(PacketKind::Audio, 2, 4, Flags::NONE, b"x");
        assert_eq!(route(&other, from(), ME, channel(3)), Route::OtherChannel);
        let away = datagram(PacketKind::Hello, 2, 4, Flags::NONE, b"Away");
        assert_eq!(
            route(&away, from(), ME, channel(3)),
            Route::Peer(PeerMessage::Hello {
                sender: PeerId::new(2),
                name: "Away".into(),
                channel: channel(4),
                addr: from(),
            })
        );
        let own = datagram(PacketKind::Audio, 1, 3, Flags::NONE, b"x");
        assert_eq!(route(&own, from(), ME, channel(3)), Route::Own);
        assert_eq!(route(b"not takkie", from(), ME, channel(3)), Route::Invalid);
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
        peer.send_to(&datagram(PacketKind::Bye, 2, 3, Flags::NONE, b""), to)
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
                sender: PeerId::new(2)
            }
        );
        let counters = thread.counters();
        drop(thread);
        assert_eq!(counters.received.load(Relaxed), 5);
        assert_eq!(counters.other_channel.load(Relaxed), 1);
        assert_eq!(counters.invalid.load(Relaxed), 1);
    }
}
