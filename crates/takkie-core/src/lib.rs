//! Pure logic for local-takkie, with no I/O.

#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod dsp;
mod ids;
pub mod jitter;
mod passphrase;
pub mod protocol;
pub mod ptt;

pub use ids::{ChannelId, InvalidChannel, PeerId, Seq};
pub use passphrase::{EmptyPassphrase, Passphrase};
