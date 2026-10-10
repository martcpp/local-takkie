//! Pure logic for local-takkie, with no I/O.

pub mod dsp;
mod ids;
pub mod jitter;
pub mod key;
mod passphrase;
pub mod peers;
pub mod protocol;
pub mod ptt;
pub mod seal;

pub use ids::{ChannelId, InvalidChannel, PeerId, Seq};
pub use passphrase::{EmptyPassphrase, Passphrase};
