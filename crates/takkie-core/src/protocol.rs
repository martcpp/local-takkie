//! Wire protocol v1 (ROADMAP.md section 3.5): every UDP datagram starts with
//! a fixed 24-byte header, followed by the payload. Multi-byte fields are
//! big-endian.

use std::ops::BitOr;

use crate::{ChannelId, PeerId, Seq};

/// The first two bytes of every packet, "TK". Anything else is dropped.
pub const MAGIC: [u8; 2] = *b"TK";

/// The protocol version this code speaks.
pub const VERSION: u8 = 1;

/// Size of the header in bytes; the payload starts right after it.
pub const HEADER_LEN: usize = 24;

/// What a packet carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PacketKind {
    /// One Opus frame.
    Audio,
    /// Keep-alive with the sender's display name.
    Hello,
    /// The sender is leaving.
    Bye,
}

impl PacketKind {
    /// The byte written in the header's `kind` field.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Audio => 1,
            Self::Hello => 2,
            Self::Bye => 3,
        }
    }
}

/// The header's `flags` byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// No flags set.
    pub const NONE: Self = Self(0);
    /// The payload is encrypted with the channel key.
    pub const ENCRYPTED: Self = Self(1);
    /// The last packet of a push-to-talk press.
    pub const END_OF_TRANSMISSION: Self = Self(1 << 1);

    /// The raw byte, as written in the header.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every flag in `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Flags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The fixed part at the start of every packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Header {
    /// What the payload is.
    pub kind: PacketKind,
    /// The channel the sender is on.
    pub channel: ChannelId,
    /// Encryption and end-of-transmission markers.
    pub flags: Flags,
    /// Who sent it.
    pub sender: PeerId,
    /// Goes up by one for every packet this sender sends, of any kind.
    pub seq: Seq,
    /// Position in the sender's audio stream, in 48 kHz samples.
    pub timestamp: u32,
}

impl Header {
    /// Writes the header into `out`, overwriting all of it, the reserved
    /// bytes included.
    pub fn encode(&self, out: &mut [u8; HEADER_LEN]) {
        out[0..2].copy_from_slice(&MAGIC);
        out[2] = VERSION;
        out[3] = self.kind.to_byte();
        out[4] = self.channel.get();
        out[5] = self.flags.bits();
        out[6..8].fill(0);
        out[8..16].copy_from_slice(&self.sender.get().to_be_bytes());
        out[16..20].copy_from_slice(&self.seq.get().to_be_bytes());
        out[20..24].copy_from_slice(&self.timestamp.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(header: &Header) -> [u8; HEADER_LEN] {
        let mut out = [0xFF; HEADER_LEN];
        header.encode(&mut out);
        out
    }

    #[test]
    fn audio_header_matches_the_spec() {
        let header = Header {
            kind: PacketKind::Audio,
            channel: ChannelId::try_from(3).unwrap(),
            flags: Flags::NONE,
            sender: PeerId::new(0x0102_0304_0506_0708),
            seq: Seq::new(0x0A0B_0C0D),
            timestamp: 0x1122_3344,
        };
        #[rustfmt::skip]
        let expected = [
            0x54, 0x4B,             // magic "TK"
            0x01,                   // version
            0x01,                   // kind: Audio
            0x03,                   // channel 3
            0x00,                   // flags
            0x00, 0x00,             // reserved
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // sender
            0x0A, 0x0B, 0x0C, 0x0D, // seq
            0x11, 0x22, 0x33, 0x44, // timestamp
        ];
        assert_eq!(encoded(&header), expected);
    }

    #[test]
    fn hello_and_bye_use_their_kind_bytes() {
        let mut header = Header {
            kind: PacketKind::Hello,
            channel: ChannelId::MAX,
            flags: Flags::NONE,
            sender: PeerId::new(u64::MAX),
            seq: Seq::new(u32::MAX),
            timestamp: 0,
        };
        #[rustfmt::skip]
        let hello = [
            0x54, 0x4B, 0x01,
            0x02,                   // kind: Hello
            0x0A,                   // channel 10
            0x00, 0x00, 0x00,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF,
            0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(encoded(&header), hello);

        header.kind = PacketKind::Bye;
        let mut bye = hello;
        bye[3] = 0x03;
        assert_eq!(encoded(&header), bye);
    }

    #[test]
    fn flags_land_in_byte_five() {
        let mut header = Header {
            kind: PacketKind::Audio,
            channel: ChannelId::MIN,
            flags: Flags::ENCRYPTED,
            sender: PeerId::new(0),
            seq: Seq::new(0),
            timestamp: 0,
        };
        assert_eq!(encoded(&header)[5], 0b01);
        header.flags = Flags::END_OF_TRANSMISSION;
        assert_eq!(encoded(&header)[5], 0b10);
        header.flags = Flags::ENCRYPTED | Flags::END_OF_TRANSMISSION;
        assert_eq!(encoded(&header)[5], 0b11);
    }

    #[test]
    fn reserved_bytes_are_always_zero() {
        let header = Header {
            kind: PacketKind::Bye,
            channel: ChannelId::MIN,
            flags: Flags::NONE,
            sender: PeerId::new(0),
            seq: Seq::new(0),
            timestamp: 0,
        };
        assert_eq!(&encoded(&header)[6..8], &[0, 0]);
    }

    #[test]
    fn flags_contains() {
        let both = Flags::ENCRYPTED | Flags::END_OF_TRANSMISSION;
        assert!(both.contains(Flags::ENCRYPTED));
        assert!(both.contains(Flags::END_OF_TRANSMISSION));
        assert!(!Flags::ENCRYPTED.contains(Flags::END_OF_TRANSMISSION));
        assert!(Flags::NONE.contains(Flags::NONE));
        assert_eq!(Flags::default(), Flags::NONE);
    }
}
