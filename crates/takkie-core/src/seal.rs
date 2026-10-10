//! Private channels: packets sealed with ChaCha20-Poly1305. The payload is
//! encrypted and the whole header is authenticated.

use chacha20poly1305::{AeadInOut, ChaCha20Poly1305, Key, KeyInit, Nonce, Tag};
use thiserror::Error;

use crate::key::ChannelKey;
use crate::protocol::{DecodeError, Flags, HEADER_LEN, Header};

/// Bytes the tag adds after the payload.
pub const TAG_LEN: usize = 16;

/// The output buffer can't hold the header, payload and tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("the buffer is too small for the sealed packet")]
pub struct NoRoom;

/// Why a packet wasn't accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum OpenError {
    /// Not a takkie header.
    #[error(transparent)]
    Header(#[from] DecodeError),
    /// The header doesn't have the encrypted flag.
    #[error("the packet isn't encrypted")]
    NotSealed,
    /// Too short to hold a tag.
    #[error("the packet is too short to hold a tag")]
    TooShort,
    /// Made with another key, or changed on the way.
    #[error("the packet doesn't match the channel key")]
    Rejected,
}

// Unique per packet under one key: ids are random per run, and seq only
// grows within a run.
fn nonce(header: &Header) -> Nonce {
    let mut bytes = [0_u8; 12];
    bytes[..8].copy_from_slice(&header.sender.get().to_be_bytes());
    bytes[8..].copy_from_slice(&header.seq.get().to_be_bytes());
    Nonce::from(bytes)
}

fn cipher(key: &ChannelKey) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(&Key::from(*key.expose_secret()))
}

/// Writes a sealed packet into `out`: the header with the encrypted flag
/// set, the encrypted payload, then the tag. Returns its length.
///
/// # Errors
/// [`NoRoom`] if `out` is shorter than header + payload + tag.
pub fn seal_packet(
    key: &ChannelKey,
    mut header: Header,
    payload: &[u8],
    out: &mut [u8],
) -> Result<usize, NoRoom> {
    let len = HEADER_LEN + payload.len() + TAG_LEN;
    let packet = out.get_mut(..len).ok_or(NoRoom)?;
    let (head, rest) = packet.split_first_chunk_mut::<HEADER_LEN>().ok_or(NoRoom)?;
    let (body, tag) = rest.split_at_mut(payload.len());
    header.flags = header.flags | Flags::ENCRYPTED;
    header.encode(head);
    body.copy_from_slice(payload);
    let made = cipher(key)
        .encrypt_inout_detached(&nonce(&header), head.as_slice(), body.into())
        .map_err(|_| NoRoom)?;
    tag.copy_from_slice(&made);
    Ok(len)
}

/// Checks and decrypts a sealed packet in place, returning its header and
/// payload. On an error the payload bytes are left encrypted.
///
/// # Errors
/// [`OpenError`] if it isn't a sealed takkie packet for this key.
pub fn open_packet<'a>(
    key: &ChannelKey,
    packet: &'a mut [u8],
) -> Result<(Header, &'a [u8]), OpenError> {
    let (header, _) = Header::decode(packet)?;
    if !header.flags.contains(Flags::ENCRYPTED) {
        return Err(OpenError::NotSealed);
    }
    let (head, rest) = packet.split_at_mut(HEADER_LEN);
    let body_len = rest.len().checked_sub(TAG_LEN).ok_or(OpenError::TooShort)?;
    let (body, tag) = rest.split_at_mut(body_len);
    let tag = Tag::try_from(&*tag).map_err(|_| OpenError::TooShort)?;
    cipher(key)
        .decrypt_inout_detached(&nonce(&header), head, (&mut *body).into(), &tag)
        .map_err(|_| OpenError::Rejected)?;
    Ok((header, body))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::protocol::PacketKind;
    use crate::{ChannelId, PeerId, Seq};

    fn key(byte: u8) -> ChannelKey {
        ChannelKey::from_bytes([byte; 32])
    }

    fn header() -> Header {
        Header {
            kind: PacketKind::Audio,
            channel: ChannelId::try_from(3).unwrap(),
            flags: Flags::END_OF_TRANSMISSION,
            sender: PeerId::new(0x0102_0304_0506_0708),
            seq: Seq::new(42),
            timestamp: 960,
        }
    }

    fn sealed(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0; HEADER_LEN + payload.len() + TAG_LEN];
        let len = seal_packet(&key(7), header(), payload, &mut out).unwrap();
        assert_eq!(len, out.len());
        out
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn a_sealed_packet_opens_with_the_same_key() {
        let mut packet = sealed(b"opus frame");
        assert_ne!(&packet[HEADER_LEN..HEADER_LEN + 10], b"opus frame");
        let (opened, payload) = open_packet(&key(7), &mut packet).unwrap();
        assert_eq!(payload, b"opus frame");
        assert_eq!(opened.flags, Flags::END_OF_TRANSMISSION | Flags::ENCRYPTED);
        assert_eq!(opened.seq, header().seq);
    }

    #[test]
    fn an_empty_payload_still_carries_a_tag() {
        let mut packet = sealed(b"");
        assert_eq!(packet.len(), HEADER_LEN + TAG_LEN);
        assert_eq!(open_packet(&key(7), &mut packet).unwrap().1, b"");
    }

    #[test]
    fn another_key_is_rejected() {
        let mut packet = sealed(b"opus frame");
        assert_eq!(
            open_packet(&key(8), &mut packet).err(),
            Some(OpenError::Rejected)
        );
    }

    #[test]
    fn changing_any_byte_is_rejected() {
        let good = sealed(b"opus frame");
        for at in 0..good.len() {
            let mut packet = good.clone();
            packet[at] ^= 0x01;
            assert!(
                open_packet(&key(7), &mut packet).is_err(),
                "a flipped bit in byte {at} went unnoticed"
            );
        }
    }

    #[test]
    fn short_and_unsealed_packets_are_refused() {
        let good = sealed(b"opus frame");
        let mut cut = good[..HEADER_LEN + TAG_LEN - 1].to_vec();
        assert_eq!(
            open_packet(&key(7), &mut cut).err(),
            Some(OpenError::TooShort)
        );
        let mut tiny = good[..10].to_vec();
        assert!(matches!(
            open_packet(&key(7), &mut tiny),
            Err(OpenError::Header(_))
        ));
        let mut clear = [0; HEADER_LEN + 20];
        header().encode((&mut clear[..HEADER_LEN]).try_into().unwrap());
        assert_eq!(
            open_packet(&key(7), &mut clear).err(),
            Some(OpenError::NotSealed)
        );
    }

    #[test]
    fn a_small_buffer_is_an_error_not_a_panic() {
        let mut out = [0; HEADER_LEN + 4 + TAG_LEN - 1];
        assert_eq!(
            seal_packet(&key(7), header(), b"opus", &mut out),
            Err(NoRoom)
        );
    }

    #[test]
    fn matches_an_independent_implementation_and_the_spec() {
        let head = "544b01010303000001020304050607080000002a000003c0";
        let rest = "81a3a25cdf1c770c1831062793ebfc6c2ca484ed427c518c4325481c0974a3bc8879";
        assert_eq!(hex(&sealed(b"takkie test vector")), format!("{head}{rest}"));
        let spec = include_str!("../../../docs/protocol.md");
        assert!(
            spec.contains(head) && spec.contains(rest),
            "docs/protocol.md doesn't list the sealed packet vector"
        );
    }

    proptest! {
        #[test]
        fn any_payload_round_trips(
            payload in proptest::collection::vec(any::<u8>(), 0..1_300),
            sender in any::<u64>(),
            seq in any::<u32>(),
            secret in any::<[u8; 32]>(),
        ) {
            let key = ChannelKey::from_bytes(secret);
            let header = Header { sender: PeerId::new(sender), seq: Seq::new(seq), ..header() };
            let mut packet = vec![0; HEADER_LEN + payload.len() + TAG_LEN];
            seal_packet(&key, header, &payload, &mut packet).unwrap();
            let (opened, plain) = open_packet(&key, &mut packet).unwrap();
            prop_assert_eq!(plain, &payload[..]);
            prop_assert_eq!(opened.sender, header.sender);
            prop_assert_eq!(opened.seq, header.seq);
        }
    }
}
