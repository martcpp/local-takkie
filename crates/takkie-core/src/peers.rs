//! Who is around.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::{ChannelId, PeerId};

/// How we learned about a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerSource {
    /// mDNS.
    Mdns,
    /// Its packets.
    Packet,
    /// Added by hand; never expires.
    Manual,
}

/// One running app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Id.
    pub id: PeerId,
    /// Display name.
    pub name: String,
    /// Packet address.
    pub addr: SocketAddr,
    /// Channel.
    pub channel: ChannelId,
    /// Last heard from.
    pub last_seen: Instant,
    /// Source.
    pub source: PeerSource,
}

/// A change for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerEvent {
    /// New peer.
    Joined(PeerId),
    /// Name, address or channel changed.
    Updated(PeerId),
    /// Left or went quiet.
    Left(PeerId),
}

/// All known peers.
#[derive(Clone, Debug, Default)]
pub struct PeerTable {
    peers: BTreeMap<PeerId, Peer>,
}

impl PeerTable {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `None` when only `last_seen` changed.
    pub fn upsert(&mut self, mut peer: Peer) -> Option<PeerEvent> {
        let Some(known) = self.peers.get_mut(&peer.id) else {
            let id = peer.id;
            self.peers.insert(id, peer);
            return Some(PeerEvent::Joined(id));
        };
        if known.source == PeerSource::Manual {
            peer.source = PeerSource::Manual;
        }
        let changed =
            known.name != peer.name || known.addr != peer.addr || known.channel != peer.channel;
        *known = peer;
        changed.then_some(PeerEvent::Updated(known.id))
    }

    /// For Bye packets.
    pub fn remove(&mut self, id: PeerId) -> Option<PeerEvent> {
        self.peers.remove(&id).map(|_| PeerEvent::Left(id))
    }

    /// Drops peers quiet for `timeout`, except manual ones.
    pub fn expire(&mut self, now: Instant, timeout: Duration) -> Vec<PeerEvent> {
        let mut left = Vec::new();
        self.peers.retain(|&id, peer| {
            let alive = peer.source == PeerSource::Manual
                || now.saturating_duration_since(peer.last_seen) < timeout;
            if !alive {
                left.push(PeerEvent::Left(id));
            }
            alive
        });
        left
    }

    /// Peers on `channel`.
    pub fn peers_on(&self, channel: ChannelId) -> impl Iterator<Item = &Peer> {
        self.peers
            .values()
            .filter(move |peer| peer.channel == channel)
    }

    /// A peer by id.
    #[must_use]
    pub fn get(&self, id: PeerId) -> Option<&Peer> {
        self.peers.get(&id)
    }

    /// Number of peers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Whether it's empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(10);

    fn peer(id: u64, channel: u8, seen: Instant) -> Peer {
        Peer {
            id: PeerId::new(id),
            name: format!("peer {id}"),
            addr: SocketAddr::from(([192, 168, 1, 10], 40_000)),
            channel: ChannelId::try_from(channel).unwrap(),
            last_seen: seen,
            source: PeerSource::Mdns,
        }
    }

    #[test]
    fn a_new_peer_joins() {
        let mut table = PeerTable::new();
        assert!(table.is_empty());
        assert_eq!(
            table.upsert(peer(1, 1, Instant::now())),
            Some(PeerEvent::Joined(PeerId::new(1)))
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn a_refresh_changes_nothing_visible() {
        let t0 = Instant::now();
        let mut table = PeerTable::new();
        table.upsert(peer(1, 1, t0));
        let later = t0 + Duration::from_secs(2);
        assert_eq!(table.upsert(peer(1, 1, later)), None);
        assert_eq!(table.get(PeerId::new(1)).map(|p| p.last_seen), Some(later));
    }

    #[test]
    fn name_address_or_channel_changes_are_updates() {
        let t0 = Instant::now();
        let mut table = PeerTable::new();
        table.upsert(peer(1, 1, t0));
        let updated = Some(PeerEvent::Updated(PeerId::new(1)));

        let mut renamed = peer(1, 1, t0);
        renamed.name = "kitchen".into();
        assert_eq!(table.upsert(renamed), updated);

        let mut moved = peer(1, 1, t0);
        moved.name = "kitchen".into();
        moved.addr = SocketAddr::from(([192, 168, 1, 11], 40_000));
        assert_eq!(table.upsert(moved), updated);

        let mut switched = table.get(PeerId::new(1)).cloned().unwrap();
        switched.channel = ChannelId::MAX;
        assert_eq!(table.upsert(switched), updated);
        assert_eq!(
            table.get(PeerId::new(1)).map(|p| p.channel),
            Some(ChannelId::MAX)
        );
    }

    #[test]
    fn quiet_peers_expire_and_fresh_ones_stay() {
        let t0 = Instant::now();
        let mut table = PeerTable::new();
        table.upsert(peer(1, 1, t0));
        table.upsert(peer(2, 1, t0 + Duration::from_secs(5)));

        assert_eq!(table.expire(t0 + Duration::from_secs(9), TIMEOUT), []);
        assert_eq!(
            table.expire(t0 + TIMEOUT, TIMEOUT),
            [PeerEvent::Left(PeerId::new(1))]
        );
        assert!(table.get(PeerId::new(2)).is_some());
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn manual_peers_never_expire_and_stay_manual() {
        let t0 = Instant::now();
        let mut table = PeerTable::new();
        let mut manual = peer(1, 1, t0);
        manual.source = PeerSource::Manual;
        table.upsert(manual);

        let mut heard = peer(1, 1, t0);
        heard.source = PeerSource::Packet;
        assert_eq!(table.upsert(heard), None);
        assert_eq!(
            table.get(PeerId::new(1)).map(|p| p.source),
            Some(PeerSource::Manual)
        );
        assert_eq!(table.expire(t0 + Duration::from_secs(3_600), TIMEOUT), []);
    }

    #[test]
    fn remove_reports_left_once() {
        let mut table = PeerTable::new();
        table.upsert(peer(1, 1, Instant::now()));
        assert_eq!(
            table.remove(PeerId::new(1)),
            Some(PeerEvent::Left(PeerId::new(1)))
        );
        assert_eq!(table.remove(PeerId::new(1)), None);
        assert!(table.is_empty());
    }

    #[test]
    fn peers_on_lists_only_that_channel() {
        let t0 = Instant::now();
        let mut table = PeerTable::new();
        for (id, channel) in [(3, 2), (1, 2), (2, 5)] {
            table.upsert(peer(id, channel, t0));
        }
        let on_two: Vec<u64> = table
            .peers_on(ChannelId::try_from(2).unwrap())
            .map(|p| p.id.get())
            .collect();
        assert_eq!(on_two, [1, 3]);
        assert_eq!(table.peers_on(ChannelId::MIN).count(), 0);
    }
}
