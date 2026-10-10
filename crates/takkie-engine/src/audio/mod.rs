/// Opus encoding and decoding.
pub mod codec;
/// Stream config negotiation.
pub mod config;
/// Device listing.
pub mod devices;
/// Fake devices for tests.
pub mod fake;
/// The hardware boundary.
pub mod io;
/// The receiving mixer.
pub mod mix;
/// Rate conversion to and from 48 kHz.
pub mod resample;
/// Device loss recovery.
pub mod session;
/// Real-time cpal streams.
pub mod stream;
/// The sending thread.
pub mod tx;

pub use io::{AudioSink, AudioSource};
