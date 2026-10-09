//! Property tests for the protocol v1 header.

use proptest::prelude::*;
use takkie_core::protocol::{Flags, HEADER_LEN, Header, PacketKind};
use takkie_core::{ChannelId, PeerId, Seq};

fn header() -> impl Strategy<Value = Header> {
    (
        prop_oneof![
            Just(PacketKind::Audio),
            Just(PacketKind::Hello),
            Just(PacketKind::Bye)
        ],
        prop::sample::select(ChannelId::ALL.to_vec()),
        any::<u8>(),
        any::<u64>(),
        any::<u32>(),
        any::<u32>(),
    )
        .prop_map(|(kind, channel, flags, sender, seq, timestamp)| Header {
            kind,
            channel,
            flags: Flags::from_bits(flags),
            sender: PeerId::new(sender),
            seq: Seq::new(seq),
            timestamp,
        })
}

fn encode(header: &Header, payload: &[u8]) -> Vec<u8> {
    let mut bytes = [0; HEADER_LEN];
    header.encode(&mut bytes);
    [&bytes[..], payload].concat()
}

proptest! {
    #[test]
    fn decode_undoes_encode(header in header(), payload in prop::collection::vec(any::<u8>(), 0..200)) {
        let bytes = encode(&header, &payload);
        prop_assert_eq!(Header::decode(&bytes), Ok((header, &payload[..])));
    }

    #[test]
    fn decoded_headers_encode_to_the_same_bytes(bytes in prop::collection::vec(any::<u8>(), HEADER_LEN..HEADER_LEN + 8)) {
        if let Ok((header, payload)) = Header::decode(&bytes) {
            prop_assert_eq!(encode(&header, payload), bytes);
        }
    }

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
        let _ = Header::decode(&bytes);
    }

    #[test]
    fn near_valid_packets_never_panic(header in header(), at in 0..HEADER_LEN, value in any::<u8>(), cut in 0..=HEADER_LEN) {
        let mut bytes = encode(&header, b"payload");
        bytes[at] = value;
        let _ = Header::decode(&bytes);
        let _ = Header::decode(&bytes[..cut]);
    }
}
