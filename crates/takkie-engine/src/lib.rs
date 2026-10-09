//! The running engine: audio devices, codec, network and discovery.

/// Microphone capture and speaker playback, with Opus.
pub mod audio;
/// The engine API.
pub mod engine;
/// The new transport, discovery and peers.
pub mod net;
/// UDP transport and mDNS discovery.
pub mod network;

pub use engine::{Engine, EngineConfig, EngineError};

pub use audio::rad::start_audio_output;
pub use audio::sad::start_mic_capture;
pub use network::mdns::Data;
pub use network::udp::{AudioBuffer, audio_udp_recv, udp_send_audio};
