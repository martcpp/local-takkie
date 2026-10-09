//! News about peers, from packets and from mDNS, and how it lands in the
//! peer table.

use std::net::SocketAddr;
use std::time::Instant;

use takkie_core::peers::{Peer, PeerEvent, PeerSource, PeerTable};
use takkie_core::{ChannelId, PeerId};

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

#[cfg(test)]
mod tests {
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
