/// Opus encoding and decoding.
pub mod codec;
/// Stream config negotiation.
pub mod config;
/// Device listing.
pub mod devices;
/// Noticing a talker in the same room.
pub mod earshot;
/// Fake devices for tests.
pub mod fake;
/// The hardware boundary.
pub mod io;
/// The send path on a recording, for listening tests.
pub mod lab;
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
/// Voice processing before the encoder.
pub mod voice;

pub use io::{AudioSink, AudioSource};
