//! The secret a channel's encryption key is derived from (ROADMAP.md
//! section 3.6).

use std::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

/// A channel passphrase. Its memory is wiped when it's dropped, and it never
/// shows up in `Debug` output. There's no `Display`, so it can't end up in a
/// log line by accident.
///
/// A channel without encryption has no passphrase at all
/// (`Option<Passphrase>`), so an empty one is rejected.
#[derive(Clone)]
pub struct Passphrase(Zeroizing<String>);

/// An empty string was given as a passphrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("a passphrase can't be empty")]
pub struct EmptyPassphrase;

impl Passphrase {
    /// Takes ownership of `secret`, so the only copy is the one wiped on drop.
    ///
    /// # Errors
    ///
    /// [`EmptyPassphrase`] if `secret` is empty.
    pub fn new(secret: String) -> Result<Self, EmptyPassphrase> {
        let secret = Zeroizing::new(secret);
        if secret.is_empty() {
            return Err(EmptyPassphrase);
        }
        Ok(Self(secret))
    }

    /// The passphrase itself, for deriving the channel key. Keep the borrow
    /// short and never log it.
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
