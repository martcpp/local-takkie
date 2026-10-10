//! Channel passphrases.

use std::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

/// A channel passphrase, wiped on drop and hidden from `Debug`.
#[derive(Clone)]
pub struct Passphrase(Zeroizing<String>);

/// The passphrase was empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("a passphrase can't be empty")]
pub struct EmptyPassphrase;

impl Passphrase {
    /// Takes ownership, so the only copy gets wiped.
    ///
    /// # Errors
    /// [`EmptyPassphrase`] if it's empty.
    pub fn new(secret: String) -> Result<Self, EmptyPassphrase> {
        let secret = Zeroizing::new(secret);
        if secret.is_empty() {
            return Err(EmptyPassphrase);
        }
        Ok(Self(secret))
    }

    /// The passphrase itself. Never log it.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_secret() {
        let passphrase = Passphrase::new("correct horse".to_string()).unwrap();
        assert_eq!(passphrase.expose_secret(), "correct horse");
    }

    #[test]
    fn rejects_an_empty_passphrase() {
        assert_eq!(Passphrase::new(String::new()).err(), Some(EmptyPassphrase));
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let passphrase = Passphrase::new("hunter2".to_string()).unwrap();
        let shown = format!("{passphrase:?} {:?}", Some(&passphrase));
        assert!(!shown.contains("hunter2"));
        assert!(shown.contains("<redacted>"));
    }
}
