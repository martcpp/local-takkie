//! Wire protocol v1 (ROADMAP.md section 3.5): every UDP datagram starts with
//! a fixed 24-byte header, followed by the payload. Multi-byte fields are
//! big-endian.

use std::ops::BitOr;

use thiserror::Error;

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

    /// The kind for a header's `kind` byte, if it's one we know.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Audio),
            2 => Some(Self::Hello),
            3 => Some(Self::Bye),
            _ => None,
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

    /// Flags from a header's raw byte. Bits this version doesn't know are
    /// kept, not rejected, so a later version can add flags without older
    /// peers dropping its packets.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

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

    /// Reads the header at the start of `packet` and returns it with the
    /// payload that follows. `packet` comes straight off the network, so
    /// every byte is checked and no input can make this panic.
    ///
    /// # Errors
    ///
    /// A [`DecodeError`] saying which check failed, in this order: length,
    /// magic, version, kind, channel, reserved bytes.
    pub fn decode(packet: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, payload)) = packet.split_first_chunk::<HEADER_LEN>() else {
            return Err(DecodeError::TooShort { len: packet.len() });
        };
        // Laid out like the wire format: magic, version, kind, channel,
        // flags, reserved, sender, seq, timestamp.
        #[rustfmt::skip]
        let [
            m0, m1, version, kind, channel, flags, r0, r1,
            s0, s1, s2, s3, s4, s5, s6, s7,
            q0, q1, q2, q3,
            t0, t1, t2, t3,
        ] = *header;

        if [m0, m1] != MAGIC {
            return Err(DecodeError::BadMagic([m0, m1]));
        }
        if version != VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let kind = PacketKind::from_byte(kind).ok_or(DecodeError::UnknownKind(kind))?;
        let channel = ChannelId::try_from(channel).map_err(|_| DecodeError::BadChannel(channel))?;
        if [r0, r1] != [0, 0] {
            return Err(DecodeError::ReservedNotZero([r0, r1]));
        }

        let header = Self {
            kind,
            channel,
            flags: Flags::from_bits(flags),
            sender: PeerId::new(u64::from_be_bytes([s0, s1, s2, s3, s4, s5, s6, s7])),
            seq: Seq::new(u32::from_be_bytes([q0, q1, q2, q3])),
            timestamp: u32::from_be_bytes([t0, t1, t2, t3]),
        };
        Ok((header, payload))
    }
}

/// Why a packet's header was rejected. The caller drops the packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// Fewer bytes than a header.
    #[error("packet is {len} bytes, shorter than the {HEADER_LEN}-byte header")]
    TooShort {
        /// How many bytes there were.
        len: usize,
    },
    /// Doesn't start with "TK", so it isn't ours.
    #[error("packet doesn't start with \"TK\" (got {0:02x?})")]
    BadMagic([u8; 2]),
    /// A protocol version this code doesn't speak.
    #[error("protocol version {0} isn't supported (this is version {VERSION})")]
    UnsupportedVersion(u8),
    /// A `kind` byte that isn't Audio, Hello or Bye.
    #[error("unknown packet kind {0}")]
    UnknownKind(u8),
    /// A channel outside 1 to 10.
    #[error("channel {0} doesn't exist")]
    BadChannel(u8),
    /// The reserved bytes must be zero in version 1.
    #[error("reserved bytes aren't zero (got {0:02x?})")]
    ReservedNotZero([u8; 2]),
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

    fn sample() -> Header {
        Header {
            kind: PacketKind::Audio,
            channel: ChannelId::try_from(7).unwrap(),
            flags: Flags::ENCRYPTED,
            sender: PeerId::new(0xDEAD_BEEF_0000_0001),
            seq: Seq::new(41),
            timestamp: 960 * 41,
        }
    }

    fn packet(header: &Header, payload: &[u8]) -> Vec<u8> {
        let mut bytes = encoded(header).to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn decode_reverses_encode_and_returns_the_payload() {
        let header = sample();
        let bytes = packet(&header, b"opus frame");
        assert_eq!(Header::decode(&bytes), Ok((header, &b"opus frame"[..])));
    }

    #[test]
    fn decode_accepts_an_empty_payload() {
        let bytes = encoded(&sample());
        assert_eq!(Header::decode(&bytes), Ok((sample(), &[][..])));
    }

    #[test]
    fn decode_keeps_unknown_flag_bits() {
        let mut bytes = encoded(&sample());
        bytes[5] = 0b1000_0011;
        let (header, _) = Header::decode(&bytes).unwrap();
        assert!(
            header
                .flags
                .contains(Flags::ENCRYPTED | Flags::END_OF_TRANSMISSION)
        );
        assert_eq!(header.flags.bits(), 0b1000_0011);
    }

    #[test]
    fn decode_rejects_a_short_packet() {
        let bytes = encoded(&sample());
        assert_eq!(
            Header::decode(&bytes[..HEADER_LEN - 1]),
            Err(DecodeError::TooShort {
                len: HEADER_LEN - 1
            })
        );
        assert_eq!(Header::decode(&[]), Err(DecodeError::TooShort { len: 0 }));
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = encoded(&sample());
        bytes[0..2].copy_from_slice(b"XK");
        assert_eq!(Header::decode(&bytes), Err(DecodeError::BadMagic(*b"XK")));
    }

    #[test]
    fn decode_rejects_another_version() {
        let mut bytes = encoded(&sample());
        bytes[2] = 2;
        assert_eq!(
            Header::decode(&bytes),
            Err(DecodeError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn decode_rejects_unknown_kinds() {
        let mut bytes = encoded(&sample());
        for kind in [0, 4, 255] {
            bytes[3] = kind;
            assert_eq!(Header::decode(&bytes), Err(DecodeError::UnknownKind(kind)));
        }
    }

    #[test]
    fn decode_rejects_bad_channels() {
        let mut bytes = encoded(&sample());
        for channel in [0, 11, 255] {
            bytes[4] = channel;
            assert_eq!(
                Header::decode(&bytes),
                Err(DecodeError::BadChannel(channel))
            );
        }
    }

    #[test]
    fn decode_rejects_nonzero_reserved_bytes() {
        let mut bytes = encoded(&sample());
        bytes[7] = 1;
        assert_eq!(
            Header::decode(&bytes),
            Err(DecodeError::ReservedNotZero([0, 1]))
        );
    }

    #[test]
    fn decode_checks_the_length_first_then_the_magic() {
        assert_eq!(Header::decode(b"no"), Err(DecodeError::TooShort { len: 2 }));
        let junk = [0xFF; HEADER_LEN];
        assert_eq!(
            Header::decode(&junk),
            Err(DecodeError::BadMagic([0xFF, 0xFF]))
        );
    }

    #[test]
    fn decode_never_panics_on_odd_input() {
        // Every length up to two headers, filled with each byte value, plus
        // a valid header with every possible value in each byte.
        for len in 0..=2 * HEADER_LEN {
            for fill in [0x00, 0x01, 0x54, 0x7F, 0x80, 0xFF] {
                let _ = Header::decode(&vec![fill; len]);
            }
        }
        let valid = encoded(&sample());
        for at in 0..HEADER_LEN {
            for value in 0..=u8::MAX {
                let mut bytes = valid;
                bytes[at] = value;
                let _ = Header::decode(&bytes);
            }
        }
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
