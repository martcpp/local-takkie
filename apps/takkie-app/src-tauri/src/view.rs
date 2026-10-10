//! What the frontend is sent: plain data it can read as JSON.

use std::time::Instant;

use serde::Serialize;
use takkie_core::PeerId;
use takkie_core::dsp::Level;
use takkie_engine::{DeviceInfo, DeviceList, EngineEvent, EngineSnapshot, PeerInfo};

/// Loudness of the latest frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LevelView {
    rms: f32,
    peak: f32,
}

impl From<Level> for LevelView {
    fn from(level: Level) -> Self {
        Self {
            rms: level.rms,
            peak: level.peak,
        }
    }
}

/// Another takkie on the network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    // Hex text: a 64-bit id doesn't fit a JavaScript number.
    id: String,
    name: String,
    channel: u8,
    addr: String,
    talking: bool,
    seen_ms: u64,
    mismatch: bool,
    muted: bool,
}

impl PeerView {
    fn at(peer: &PeerInfo, now: Instant) -> Self {
        let seen = now.saturating_duration_since(peer.last_seen);
        Self {
            id: peer.id.to_string(),
            name: peer.name.clone(),
            channel: peer.channel.get(),
            addr: peer.addr.to_string(),
            talking: peer.talking,
            seen_ms: u64::try_from(seen.as_millis()).unwrap_or(u64::MAX),
            mismatch: peer.mismatch,
            muted: peer.muted,
        }
    }
}

/// Everything the main screen draws.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotView {
    id: String,
    channel: u8,
    private: bool,
    transmitting: bool,
    muted: bool,
    beeps: bool,
    volume: f32,
    mic: LevelView,
    speaker: LevelView,
    buffer_ms: u32,
    peers: Vec<PeerView>,
}

impl SnapshotView {
    /// The snapshot as the frontend sees it at `now`.
    pub fn at(snapshot: &EngineSnapshot, now: Instant) -> Self {
        Self {
            id: snapshot.id.to_string(),
            channel: snapshot.channel.get(),
            private: snapshot.private,
            transmitting: snapshot.transmitting,
            muted: snapshot.muted,
            beeps: snapshot.beeps,
            volume: snapshot.volume,
            mic: snapshot.mic.into(),
            speaker: snapshot.speaker.into(),
            buffer_ms: snapshot.buffer_ms,
            peers: snapshot
                .peers
                .iter()
                .map(|peer| PeerView::at(peer, now))
                .collect(),
        }
    }
}

/// Something that just happened, for the frontend to react to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EventView {
    /// A peer appeared.
    PeerJoined { peer: PeerView },
    /// A peer's name, channel or address changed.
    PeerUpdated { peer: PeerView },
    /// A peer left or went quiet.
    PeerLeft { id: String },
    /// A peer's voice started playing.
    TalkStarted { id: String },
    /// A peer's voice stopped.
    TalkStopped { id: String },
    /// A peer on our channel uses another passphrase.
    WrongPassphrase { id: String },
    /// A microphone or speaker is running.
    DeviceStarted {
        direction: String,
        description: String,
    },
    /// A microphone or speaker went away.
    DeviceLost { direction: String },
    /// Worth showing to the user.
    Warning { text: String },
}

impl EventView {
    /// The event as the frontend sees it, or `None` for one this app
    /// doesn't know yet.
    pub fn at(event: &EngineEvent, now: Instant) -> Option<Self> {
        let id = |id: &PeerId| id.to_string();
        Some(match event {
            EngineEvent::PeerJoined(peer) => Self::PeerJoined {
                peer: PeerView::at(peer, now),
            },
            EngineEvent::PeerUpdated(peer) => Self::PeerUpdated {
                peer: PeerView::at(peer, now),
            },
            EngineEvent::PeerLeft(peer) => Self::PeerLeft { id: id(peer) },
            EngineEvent::TalkStarted(peer) => Self::TalkStarted { id: id(peer) },
            EngineEvent::TalkStopped(peer) => Self::TalkStopped { id: id(peer) },
            EngineEvent::WrongPassphrase(peer) => Self::WrongPassphrase { id: id(peer) },
            EngineEvent::DeviceStarted {
                direction,
                description,
            } => Self::DeviceStarted {
                direction: direction.to_string(),
                description: description.clone(),
            },
            EngineEvent::DeviceLost(direction) => Self::DeviceLost {
                direction: direction.to_string(),
            },
            EngineEvent::Warning(text) => Self::Warning { text: text.clone() },
            _ => return None,
        })
    }
}

/// A microphone or speaker to choose from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    name: String,
    is_default: bool,
}

impl From<&DeviceInfo> for DeviceView {
    fn from(device: &DeviceInfo) -> Self {
        Self {
            name: device.name.clone(),
            is_default: device.is_default,
        }
    }
}

/// The microphones and speakers on this machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DevicesView {
    inputs: Vec<DeviceView>,
    outputs: Vec<DeviceView>,
}

impl From<&DeviceList> for DevicesView {
    fn from(devices: &DeviceList) -> Self {
        Self {
            inputs: devices.inputs.iter().map(DeviceView::from).collect(),
            outputs: devices.outputs.iter().map(DeviceView::from).collect(),
        }
    }
}

/// Reads a peer id as [`PeerView`] writes it.
pub fn peer_id(text: &str) -> Option<PeerId> {
    let hex = text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if text.is_empty() || text.len() > 16 || !hex {
        return None;
    }
    u64::from_str_radix(text, 16).ok().map(PeerId::new)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use takkie_core::ChannelId;
    use takkie_engine::{Direction, EngineStats};

    use super::*;

    fn peer(id: u64, seen: Instant) -> PeerInfo {
        PeerInfo {
            id: PeerId::new(id),
            name: "Kitchen".into(),
            channel: ChannelId::try_from(4).unwrap(),
            addr: SocketAddr::from(([192, 168, 1, 5], 40_000)),
            talking: true,
            last_seen: seen,
            mismatch: false,
            muted: true,
        }
    }

    fn snapshot(peers: Vec<PeerInfo>) -> EngineSnapshot {
        EngineSnapshot {
            id: PeerId::new(0xAB),
            channel: ChannelId::try_from(4).unwrap(),
            private: true,
            transmitting: false,
            muted: false,
            beeps: true,
            volume: 0.5,
            mic: Level {
                rms: 0.1,
                peak: 0.2,
            },
            speaker: Level::default(),
            buffer_ms: 60,
            peers,
            stats: EngineStats::default(),
        }
    }

    #[test]
    fn a_snapshot_becomes_camel_case_json_with_hex_ids() {
        let now = Instant::now();
        let seen = now - Duration::from_millis(1_500);
        let view = SnapshotView::at(&snapshot(vec![peer(u64::MAX, seen)]), now);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["id"], "00000000000000ab");
        assert_eq!(json["channel"], 4);
        assert_eq!(json["private"], true);
        assert_eq!(json["beeps"], true);
        assert_eq!(json["bufferMs"], 60);
        assert_eq!(json["mic"]["peak"], 0.2_f32);
        let peer = &json["peers"][0];
        assert_eq!(peer["id"], "ffffffffffffffff");
        assert_eq!(peer["name"], "Kitchen");
        assert_eq!(peer["addr"], "192.168.1.5:40000");
        assert_eq!(peer["seenMs"], 1_500);
        assert_eq!(peer["talking"], true);
        assert_eq!(peer["muted"], true);
        assert_eq!(peer["mismatch"], false);
    }

    fn event(event: &EngineEvent) -> serde_json::Value {
        serde_json::to_value(EventView::at(event, Instant::now())).unwrap()
    }

    #[test]
    fn every_event_is_tagged_with_its_type() {
        let now = Instant::now();
        let id = PeerId::new(0xAB);
        let joined = event(&EngineEvent::PeerJoined(peer(0xAB, now)));
        assert_eq!(joined["type"], "peerJoined");
        assert_eq!(joined["peer"]["id"], "00000000000000ab");
        assert_eq!(joined["peer"]["name"], "Kitchen");
        let updated = event(&EngineEvent::PeerUpdated(peer(0xAB, now)));
        assert_eq!(updated["type"], "peerUpdated");
        assert_eq!(updated["peer"]["channel"], 4);

        for (sent, kind) in [
            (EngineEvent::PeerLeft(id), "peerLeft"),
            (EngineEvent::TalkStarted(id), "talkStarted"),
            (EngineEvent::TalkStopped(id), "talkStopped"),
            (EngineEvent::WrongPassphrase(id), "wrongPassphrase"),
        ] {
            let json = event(&sent);
            assert_eq!(json["type"], kind);
            assert_eq!(json["id"], "00000000000000ab");
        }

        let started = event(&EngineEvent::DeviceStarted {
            direction: Direction::Input,
            description: "Headset (48000 Hz)".into(),
        });
        assert_eq!(started["type"], "deviceStarted");
        assert_eq!(started["direction"], "input");
        assert_eq!(started["description"], "Headset (48000 Hz)");
        let lost = event(&EngineEvent::DeviceLost(Direction::Output));
        assert_eq!(lost["type"], "deviceLost");
        assert_eq!(lost["direction"], "output");
        let warning = event(&EngineEvent::Warning("no such microphone".into()));
        assert_eq!(warning["type"], "warning");
        assert_eq!(warning["text"], "no such microphone");
    }

    #[test]
    fn a_peer_seen_in_the_future_was_seen_just_now() {
        let now = Instant::now();
        let view = PeerView::at(&peer(1, now + Duration::from_secs(1)), now);
        assert_eq!(view.seen_ms, 0);
    }

    #[test]
    fn a_peer_id_survives_the_trip_to_the_frontend_and_back() {
        for raw in [0, 1, 0xAB, u64::MAX] {
            let id = PeerId::new(raw);
            assert_eq!(peer_id(&id.to_string()), Some(id));
        }
        assert_eq!(peer_id("AB"), Some(PeerId::new(0xAB)));
    }

    #[test]
    fn text_that_is_not_a_peer_id_is_refused() {
        for text in ["", "xyz", "12345678901234567", "-1", "+1", " 1"] {
            assert_eq!(peer_id(text), None, "{text:?}");
        }
    }

    #[test]
    fn devices_keep_their_name_and_default_mark() {
        let device = |name: &str, is_default| DeviceInfo {
            name: name.into(),
            is_default,
            rates: None,
            max_channels: 2,
            formats: Vec::new(),
        };
        let list = DeviceList {
            inputs: vec![device("Headset", true), device("Webcam", false)],
            outputs: vec![device("Speakers", true)],
        };
        let json = serde_json::to_value(DevicesView::from(&list)).unwrap();
        assert_eq!(json["inputs"][0]["name"], "Headset");
        assert_eq!(json["inputs"][0]["isDefault"], true);
        assert_eq!(json["inputs"][1]["isDefault"], false);
        assert_eq!(json["outputs"][0]["name"], "Speakers");
    }
}
