//! The running engine: audio devices, codec, network and discovery.

/// Microphone capture and speaker playback, with Opus.
pub mod audio;
/// The engine API.
pub mod engine;
/// Transport, discovery and peers.
pub mod net;
mod threads;

pub use audio::devices::{DeviceInfo, DeviceList, Direction};
pub use engine::{
    Engine, EngineConfig, EngineError, EngineEvent, EngineSnapshot, EngineStats, PeerInfo,
};
