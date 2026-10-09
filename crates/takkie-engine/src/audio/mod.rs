/// Stream config negotiation.
pub mod config;
/// Fake devices for tests.
pub mod fake;
/// The hardware boundary.
pub mod io;
/// Speaker side: decodes received Opus packets and plays them.
pub mod rad;
/// Microphone side: encodes 20 ms Opus frames and sends them to peers.
pub mod sad;

pub use io::{AudioSink, AudioSource};
