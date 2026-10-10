#![no_main]

use libfuzzer_sys::fuzz_target;
use takkie_core::protocol::{HEADER_LEN, Header};

fuzz_target!(|data: &[u8]| {
    if let Ok((header, payload)) = Header::decode(data) {
        let mut bytes = [0; HEADER_LEN];
        header.encode(&mut bytes);
        assert_eq!([&bytes[..], payload].concat(), data);
    }
});
