//! Channel keys: a passphrase turned into the key for one channel.

use std::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{ChannelId, Passphrase};

/// Key size in bytes.
pub const KEY_LEN: usize = 32;

const MEMORY_KIB: u32 = 19 * 1024;
const PASSES: u32 = 2;
const LANES: u32 = 1;

/// Argon2 refused the parameters or ran out of memory.
#[derive(Debug, Error)]
#[error("couldn't derive the channel key: {0}")]
pub struct KeyError(argon2::Error);

/// The key for one channel, wiped on drop and hidden from `Debug`.
#[derive(Clone)]
pub struct ChannelKey(Zeroizing<[u8; KEY_LEN]>);

impl ChannelKey {
    /// Derives the key with Argon2id (19 MiB, 2 passes). The channel is in
    /// the salt, so one passphrase gives a different key on each channel.
    ///
    /// Slow on purpose: call it once per join, never on an audio thread.
    ///
    /// # Errors
    /// [`KeyError`] if the 19 MiB can't be allocated.
    pub fn derive(passphrase: &Passphrase, channel: ChannelId) -> Result<Self, KeyError> {
        let params = Params::new(MEMORY_KIB, PASSES, LANES, Some(KEY_LEN)).map_err(KeyError)?;
        let salt = format!("local-takkie/v1/channel/{channel}");
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(
                passphrase.expose_secret().as_bytes(),
                salt.as_bytes(),
                key.as_mut_slice(),
            )
            .map_err(KeyError)?;
        Ok(Self(key))
    }

    /// A key from raw bytes, for tests and test vectors.
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The key itself. Never log it.
    #[must_use]
    pub fn expose_secret(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for ChannelKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChannelKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(passphrase: &str, channel: u8) -> ChannelKey {
        let passphrase = Passphrase::new(passphrase.to_string()).unwrap();
        ChannelKey::derive(&passphrase, ChannelId::try_from(channel).unwrap()).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn the_same_passphrase_and_channel_give_the_same_key() {
        assert_eq!(
            key("correct horse", 3).expose_secret(),
            key("correct horse", 3).expose_secret()
        );
    }

    #[test]
    fn another_channel_or_passphrase_gives_another_key() {
        let base = key("correct horse", 3);
        assert_ne!(
            base.expose_secret(),
            key("correct horse", 4).expose_secret()
        );
        assert_ne!(
            base.expose_secret(),
            key("correct horsf", 3).expose_secret()
        );
    }

    #[test]
    fn debug_never_shows_the_key() {
        let key = ChannelKey::from_bytes([0xAB; KEY_LEN]);
        let shown = format!("{key:?}");
        assert_eq!(shown, "ChannelKey(<redacted>)");
        assert!(!shown.contains("ab"));
    }

    #[test]
    fn matches_the_reference_argon2_implementation_and_the_spec() {
        let spec = include_str!("../../../docs/protocol.md");
        for (passphrase, channel, expected) in [
            (
                "correct horse battery staple",
                1,
                "3a5f7f18f2bccbb3ef7b818a6aab748633b2a6aace8f0fcd66c4a2b259f78840",
            ),
            (
                "takkie",
                10,
                "2a1fa1e92997e7d4e20a36fd63c0ca893b6b63577084d3f5cba5e75cc929cb05",
            ),
        ] {
            assert_eq!(hex(key(passphrase, channel).expose_secret()), expected);
            assert!(
                spec.contains(&format!("\"{passphrase}\", {channel}:")) && spec.contains(expected),
                "docs/protocol.md doesn't list the {passphrase} vector"
            );
        }
    }
}
