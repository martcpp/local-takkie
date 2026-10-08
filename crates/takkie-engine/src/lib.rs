//! The running engine: audio devices, codec, network and discovery.

/// Microphone capture and speaker playback, with Opus.
pub mod audio;
/// UDP transport and mDNS discovery.
pub mod network;

pub use audio::rad::start_audio_output;
pub use audio::sad::start_mic_capture;
pub use network::mdns::Data;
pub use network::udp::{AudioBuffer, audio_udp_recv, udp_send_audio};
