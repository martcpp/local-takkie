// local-takkie: LAN walkie-talkie library
// This module exposes the library functionality for both the binary and tests

pub mod audio;
pub mod network;
pub mod ui;

// Re-export commonly used types
pub use network::udp::{AudioBuffer, audio_udp_recv, udp_send_audio};
pub use network::mdns::Data;
pub use audio::rad::start_audio_output;
pub use audio::sad::start_mic_capture;
pub use ui::tui::{AppState, run_tui};
