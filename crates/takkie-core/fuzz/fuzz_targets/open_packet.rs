#![no_main]

use libfuzzer_sys::fuzz_target;
use takkie_core::key::ChannelKey;
use takkie_core::seal::{open_packet, seal_packet};

fuzz_target!(|data: &[u8]| {
    let key = ChannelKey::from_bytes([7; 32]);
    let mut packet = data.to_vec();
    if let Ok((header, payload)) = open_packet(&key, &mut packet) {
        let mut again = vec![0; data.len()];
        let len = seal_packet(&key, header, payload, &mut again).expect("same size as the input");
        assert_eq!(&again[..len], data);
    }
});
