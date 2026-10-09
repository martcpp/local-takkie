//! Pure logic for local-takkie: wire protocol, jitter buffer, DSP and the PTT
//! state machine. No I/O, so everything here is easy to test.

#![deny(clippy::unwrap_used, clippy::expect_used)]

mod ids;
mod passphrase;

pub use ids::{ChannelId, InvalidChannel, PeerId, Seq};
pub use passphrase::{EmptyPassphrase, Passphrase};
