//! News about peers, from packets and from mDNS, and the thread that keeps
//! the peer table: who we send to, and who has gone quiet.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use takkie_core::peers::{Peer, PeerEvent, PeerSource, PeerTable};
use takkie_core::{ChannelId, PeerId};

use crate::audio::mix::MixInput;
use crate::threads::{STOP_WITHIN, join_within};

/// A peer silent this long is dropped.
pub const PEER_TIMEOUT: Duration = Duration::from_secs(10);

const CHECK_EVERY: Duration = Duration::from_millis(500);

/// What the peer table needs to hear about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerMessage {
    /// A keep-alive with the sender's name.
    Hello {
        /// Who.
        sender: PeerId,
        /// Display name.
        name: String,
        /// Their channel.
        channel: ChannelId,
        /// Where it came from.
        addr: SocketAddr,
    },
    /// Audio arrived, so they're alive at `addr`.
    Heard {
        /// Who.
        sender: PeerId,
        /// Their channel.
        channel: ChannelId,
        /// Where it came from.
        addr: SocketAddr,
    },
    /// They're leaving.
    Bye {
        /// Who.
        sender: PeerId,
    },
    /// Found or changed over mDNS.
    Announced {
        /// Who.
        sender: PeerId,
        /// Display name.
        name: String,
        /// Their channel.
        channel: ChannelId,
        /// Their announced address.
        addr: SocketAddr,
    },
    /// Gone from mDNS.
    Withdrawn {
        /// Who.
        sender: PeerId,
    },
}

/// Applies `message` to `table`, returning what the UI should hear.
pub fn apply(table: &mut PeerTable, message: PeerMessage, now: Instant) -> Option<PeerEvent> {
    let known = |id: PeerId| table.get(id);
    let peer = match message {
        PeerMessage::Hello {
            sender,
            name,
            channel,
            addr,
        } => Peer {
            id: sender,
            name,
            addr,
            channel,
            last_seen: now,
            source: known(sender).map_or(PeerSource::Packet, |peer| peer.source),
        },
        PeerMessage::Heard {
            sender,
            channel,
            addr,
        } => Peer {
            id: sender,
            name: known(sender)
                .map(|peer| peer.name.clone())
                .unwrap_or_default(),
            addr,
            channel,
            last_seen: now,
            source: known(sender).map_or(PeerSource::Packet, |peer| peer.source),
        },
        PeerMessage::Announced {
            sender,
            name,
            channel,
            addr,
        } => Peer {
            id: sender,
            name,
            // Packets already got through on the address we have, so keep it.
            addr: known(sender).map_or(addr, |peer| peer.addr),
            channel,
            last_seen: now,
            source: known(sender).map_or(PeerSource::Mdns, |peer| peer.source),
        },
        PeerMessage::Bye { sender } => return table.remove(sender),
        PeerMessage::Withdrawn { sender } => {
            if known(sender).is_some_and(|peer| peer.source == PeerSource::Manual) {
                return None;
            }
            return table.remove(sender);
        }
    };
    table.upsert(peer)
}

/// The peer table, and the send list for our channel that follows it.
pub struct Peers {
    table: PeerTable,
    channel: Arc<AtomicU8>,
    targets: Arc<ArcSwap<Vec<SocketAddr>>>,
    view: Arc<ArcSwap<Vec<Peer>>>,
    timeout: Duration,
}

impl Peers {
    /// An empty table that keeps `targets` set to the peers on `channel`
    /// and drops peers quiet for `timeout`.
    #[must_use]
    pub fn new(
        channel: Arc<AtomicU8>,
        targets: Arc<ArcSwap<Vec<SocketAddr>>>,
        timeout: Duration,
    ) -> Self {
        Self {
            table: PeerTable::new(),
            channel,
            targets,
            view: Arc::default(),
            timeout,
        }
    }

    /// Every peer, republished after each change, for readers on other
    /// threads.
    #[must_use]
    pub fn view(&self) -> Arc<ArcSwap<Vec<Peer>>> {
        Arc::clone(&self.view)
    }

    /// The table.
    #[must_use]
    pub fn table(&self) -> &PeerTable {
        &self.table
    }

    /// Applies one piece of news.
    pub fn handle(&mut self, message: PeerMessage, now: Instant) -> Option<PeerEvent> {
        let event = apply(&mut self.table, message, now);
        self.publish();
        event
    }

    /// Drops quiet peers. Also picks up a channel change.
    pub fn expire(&mut self, now: Instant) -> Vec<PeerEvent> {
        let left = self.table.expire(now, self.timeout);
        self.publish();
        left
    }

    fn publish(&self) {
        self.view
            .store(Arc::new(self.table.iter().cloned().collect()));
        let Ok(channel) = ChannelId::try_from(self.channel.load(Relaxed)) else {
            return;
        };
        let wanted: Vec<SocketAddr> = self.table.peers_on(channel).map(|peer| peer.addr).collect();
        if **self.targets.load() != wanted {
            self.targets.store(Arc::new(wanted));
        }
    }
}

/// Where the peer thread reports.
pub struct PeerOutputs {
    /// Joins, changes and leaves, for the UI.
    pub events: Sender<PeerEvent>,
    /// Leaves, so the mixer can drop their decoders.
    pub mixer: Sender<MixInput>,
}

/// The running peer thread. Dropping it stops the thread.
pub struct PeerThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PeerThread {
    /// Feeds `news` into `peers` and expires quiet peers twice a second.
    ///
    /// # Errors
    /// The OS error if the thread can't start.
    pub fn spawn(
        news: Receiver<PeerMessage>,
        mut peers: Peers,
        outputs: PeerOutputs,
    ) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-peers".into())
            .spawn(move || {
                let mut next_check = Instant::now() + CHECK_EVERY;
                while !stopping.load(Relaxed) {
                    let mut events = Vec::new();
                    match news.recv_timeout(Duration::from_millis(100)) {
                        Ok(message) => events.extend(peers.handle(message, Instant::now())),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                    let now = Instant::now();
                    if now >= next_check {
                        events.extend(peers.expire(now));
                        next_check = now + CHECK_EVERY;
                    }
                    for event in events {
                        if let PeerEvent::Left(sender) = event {
                            let _ = outputs.mixer.send(MixInput::Left(sender));
                        }
                        let _ = outputs.events.send(event);
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for PeerThread {
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

    use super::*;

    const KITCHEN: PeerId = PeerId::new(7);

    fn ch(n: u8) -> ChannelId {
        ChannelId::try_from(n).unwrap()
    }

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::from(([192, 168, 1, last], 40_000))
    }

    fn announced(name: &str, channel: u8, at: u8) -> PeerMessage {
        PeerMessage::Announced {
            sender: KITCHEN,
            name: name.into(),
            channel: ch(channel),
            addr: addr(at),
        }
    }

    fn hello(sender: PeerId, channel: u8, at: u8) -> PeerMessage {
        PeerMessage::Hello {
            sender,
            name: "x".into(),
            channel: ch(channel),
            addr: addr(at),
        }
    }

    fn peers_on(
        channel: u8,
        timeout: Duration,
    ) -> (Peers, Arc<AtomicU8>, Arc<ArcSwap<Vec<SocketAddr>>>) {
        let channel = Arc::new(AtomicU8::new(channel));
        let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let peers = Peers::new(Arc::clone(&channel), Arc::clone(&targets), timeout);
        (peers, channel, targets)
    }

    #[test]
    fn the_send_list_follows_our_channel() {
        let (mut peers, channel, targets) = peers_on(2, PEER_TIMEOUT);
        let now = Instant::now();
        peers.handle(hello(PeerId::new(1), 2, 1), now);
        peers.handle(hello(PeerId::new(2), 5, 2), now);
        peers.handle(hello(PeerId::new(3), 2, 3), now);
        assert_eq!(**targets.load(), [addr(1), addr(3)]);
        assert_eq!(peers.view().load().len(), 3);
        channel.store(5, Relaxed);
        peers.expire(now);
        assert_eq!(**targets.load(), [addr(2)]);
        peers.handle(
            PeerMessage::Bye {
                sender: PeerId::new(2),
            },
            now,
        );
        assert!(targets.load().is_empty());
    }

    #[test]
    fn quiet_peers_expire_and_hello_keeps_them() {
        let (mut peers, _, targets) = peers_on(2, PEER_TIMEOUT);
        let start = Instant::now();
        peers.handle(hello(PeerId::new(1), 2, 1), start);
        peers.handle(hello(PeerId::new(2), 2, 2), start);
        let later = start + Duration::from_secs(8);
        peers.handle(hello(PeerId::new(2), 2, 2), later);
        assert!(peers.expire(start + Duration::from_secs(9)).is_empty());
        assert_eq!(
            peers.expire(start + PEER_TIMEOUT),
            [PeerEvent::Left(PeerId::new(1))]
        );
        assert_eq!(**targets.load(), [addr(2)]);
        assert_eq!(peers.table().len(), 1);
    }

    #[test]
    fn the_thread_reports_joins_and_tells_the_mixer_who_went_quiet() {
        let (peers, _, targets) = peers_on(2, Duration::from_millis(300));
        let (news, news_in) = unbounded();
        let (events, events_out) = unbounded();
        let (mixer, mixer_out) = unbounded();
        let thread = PeerThread::spawn(news_in, peers, PeerOutputs { events, mixer }).unwrap();

        news.send(hello(KITCHEN, 2, 4)).unwrap();
        let wait = Duration::from_secs(3);
        assert_eq!(
            events_out.recv_timeout(wait).unwrap(),
            PeerEvent::Joined(KITCHEN)
        );
        assert_eq!(**targets.load(), [addr(4)]);

        let quiet = Instant::now();
        assert_eq!(
            events_out.recv_timeout(wait).unwrap(),
            PeerEvent::Left(KITCHEN)
        );
        assert!(quiet.elapsed() < Duration::from_millis(1_000));
        assert_eq!(
            mixer_out.recv_timeout(wait).unwrap(),
            MixInput::Left(KITCHEN)
        );
        assert!(targets.load().is_empty());
        drop(thread);
    }

    #[test]
    fn mdns_join_update_and_leave() {
        let mut table = PeerTable::new();
        let now = Instant::now();
        assert_eq!(
            apply(&mut table, announced("Kitchen", 2, 5), now),
            Some(PeerEvent::Joined(KITCHEN))
        );
        assert_eq!(apply(&mut table, announced("Kitchen", 2, 5), now), None);
        assert_eq!(
            apply(&mut table, announced("Kitchen", 3, 5), now),
            Some(PeerEvent::Updated(KITCHEN))
        );
        let peer = table.get(KITCHEN).unwrap();
        assert_eq!(peer.channel, ch(3));
        assert_eq!(peer.source, PeerSource::Mdns);
        assert_eq!(
            apply(&mut table, PeerMessage::Withdrawn { sender: KITCHEN }, now),
            Some(PeerEvent::Left(KITCHEN))
        );
        assert!(table.is_empty());
    }

    #[test]
    fn the_packet_address_wins_over_the_announced_one() {
        let mut table = PeerTable::new();
        let now = Instant::now();
        apply(&mut table, announced("Kitchen", 2, 5), now);
        let hello = PeerMessage::Hello {
            sender: KITCHEN,
            name: "Kitchen".into(),
            channel: ch(2),
            addr: addr(9),
        };
        assert_eq!(
            apply(&mut table, hello, now),
            Some(PeerEvent::Updated(KITCHEN))
        );
        assert_eq!(apply(&mut table, announced("Kitchen", 2, 5), now), None);
        let peer = table.get(KITCHEN).unwrap();
        assert_eq!(peer.addr, addr(9));
        assert_eq!(peer.source, PeerSource::Mdns);
    }

    #[test]
    fn audio_from_a_stranger_adds_them_without_a_name() {
        let mut table = PeerTable::new();
        let heard = PeerMessage::Heard {
            sender: KITCHEN,
            channel: ch(1),
            addr: addr(4),
        };
        assert_eq!(
            apply(&mut table, heard, Instant::now()),
            Some(PeerEvent::Joined(KITCHEN))
        );
        let peer = table.get(KITCHEN).unwrap();
        assert_eq!(peer.name, "");
        assert_eq!(peer.source, PeerSource::Packet);
    }

    #[test]
    fn audio_keeps_the_name_and_refreshes_last_seen() {
        let mut table = PeerTable::new();
        let start = Instant::now();
        apply(&mut table, announced("Kitchen", 1, 4), start);
        let later = start + std::time::Duration::from_secs(5);
        let heard = PeerMessage::Heard {
            sender: KITCHEN,
            channel: ch(1),
            addr: addr(4),
        };
        assert_eq!(apply(&mut table, heard, later), None);
        let peer = table.get(KITCHEN).unwrap();
        assert_eq!(peer.name, "Kitchen");
        assert_eq!(peer.last_seen, later);
    }

    #[test]
    fn bye_removes_and_mdns_leaves_manual_peers_alone() {
        let mut table = PeerTable::new();
        let now = Instant::now();
        table.upsert(Peer {
            id: KITCHEN,
            name: "Kitchen".into(),
            addr: addr(4),
            channel: ch(1),
            last_seen: now,
            source: PeerSource::Manual,
        });
        assert_eq!(
            apply(&mut table, PeerMessage::Withdrawn { sender: KITCHEN }, now),
            None
        );
        assert_eq!(
            apply(&mut table, PeerMessage::Bye { sender: KITCHEN }, now),
            Some(PeerEvent::Left(KITCHEN))
        );
        assert_eq!(
            apply(&mut table, PeerMessage::Bye { sender: KITCHEN }, now),
            None
        );
    }
}
