//! Identifiers that travel in every packet header (ROADMAP.md section 3.5).

use std::fmt;

use thiserror::Error;

/// One of the ten channels, numbered 1 to 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(u8);

/// A number that isn't a valid channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("channel {0} doesn't exist; channels are {min} to {max}", min = ChannelId::MIN.0, max = ChannelId::MAX.0)]
pub struct InvalidChannel(pub u8);

impl ChannelId {
    /// The lowest channel.
    pub const MIN: Self = Self(1);
    /// The highest channel.
    pub const MAX: Self = Self(10);
    /// Every channel, lowest first.
    pub const ALL: [Self; 10] = [
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
        Self(8),
        Self(9),
        Self(10),
    ];

    /// The channel number, 1 to 10.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for ChannelId {
    type Error = InvalidChannel;

    /// # Errors
    ///
    /// [`InvalidChannel`] if `value` is outside 1 to 10.
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if (Self::MIN.0..=Self::MAX.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidChannel(value))
        }
    }
}

impl From<ChannelId> for u8 {
    fn from(channel: ChannelId) -> Self {
        channel.0
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifies one running app on the network. Picked at random at startup,
/// so it also tells a restarted app apart from its previous run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerId(u64);

impl PeerId {
    /// Wraps a raw id, for example one read from a packet header.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw id, as written in a packet header.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PeerId {
    /// Sixteen hex digits, so every id has the same width in logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// A packet sequence number. It goes up by one per packet and wraps around
/// after `u32::MAX`.
///
/// There's deliberately no `Ord`: with wrap-around, "newer than" isn't
/// transitive, so use [`Seq::is_newer_than`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seq(u32);

impl Seq {
    /// Wraps a raw sequence number, for example one read from a packet header.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw number, as written in a packet header.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The number after this one, wrapping from `u32::MAX` to 0.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// Whether `self` comes after `other`, allowing for wrap-around: anything
    /// up to half the number space ahead counts as newer (RFC 1982 serial
    /// number arithmetic). Exactly half apart is ambiguous, so neither side
    /// is newer.
    #[must_use]
    pub const fn is_newer_than(self, other: Self) -> bool {
        let ahead = self.0.wrapping_sub(other.0);
        ahead != 0 && ahead < 1 << 31
    }
}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_one_to_ten_are_valid() {
        for n in 1..=10 {
            let channel = ChannelId::try_from(n).unwrap();
            assert_eq!(channel.get(), n);
            assert_eq!(u8::from(channel), n);
        }
    }

    #[test]
    fn channels_outside_one_to_ten_are_rejected() {
        for n in [0, 11, 12, 100, 255] {
            assert_eq!(ChannelId::try_from(n), Err(InvalidChannel(n)));
        }
    }

    #[test]
    fn invalid_channel_message_names_the_range() {
        assert_eq!(
            InvalidChannel(11).to_string(),
            "channel 11 doesn't exist; channels are 1 to 10"
        );
    }

    #[test]
    fn all_lists_every_channel_in_order() {
        let numbers: Vec<u8> = ChannelId::ALL.iter().map(|c| c.get()).collect();
        assert_eq!(numbers, (1..=10).collect::<Vec<_>>());
        assert_eq!(ChannelId::ALL.first(), Some(&ChannelId::MIN));
        assert_eq!(ChannelId::ALL.last(), Some(&ChannelId::MAX));
    }

    #[test]
    fn channel_displays_as_its_number() {
        assert_eq!(ChannelId::MIN.to_string(), "1");
        assert_eq!(ChannelId::MAX.to_string(), "10");
    }

    #[test]
    fn peer_id_displays_as_sixteen_hex_digits() {
        assert_eq!(PeerId::new(0xab).to_string(), "00000000000000ab");
        assert_eq!(PeerId::new(u64::MAX).to_string(), "ffffffffffffffff");
        assert_eq!(PeerId::new(42).get(), 42);
    }

    #[test]
    fn next_seq_wraps_around() {
        assert_eq!(Seq::new(7).next(), Seq::new(8));
        assert_eq!(Seq::new(u32::MAX).next(), Seq::new(0));
    }

    #[test]
    fn newer_seq_without_wrap() {
        assert!(Seq::new(2).is_newer_than(Seq::new(1)));
        assert!(!Seq::new(1).is_newer_than(Seq::new(2)));
        assert!(!Seq::new(5).is_newer_than(Seq::new(5)));
    }

    #[test]
    fn newer_seq_across_the_wrap() {
        assert!(Seq::new(0).is_newer_than(Seq::new(u32::MAX)));
        assert!(Seq::new(10).is_newer_than(Seq::new(u32::MAX - 10)));
        assert!(!Seq::new(u32::MAX).is_newer_than(Seq::new(0)));
    }

    #[test]
    fn half_the_range_apart_is_neither_newer() {
        let a = Seq::new(0);
        let b = Seq::new(1 << 31);
        assert!(!a.is_newer_than(b));
        assert!(!b.is_newer_than(a));
        assert!(Seq::new((1 << 31) - 1).is_newer_than(a));
    }
}
