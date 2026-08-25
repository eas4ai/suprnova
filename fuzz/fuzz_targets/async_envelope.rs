#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::async_updates::{
    AsyncCodecLimits, decode_async_envelope, encode_async_envelope,
};

mod support;

fuzz_target!(|bytes: &[u8]| {
    let limits = AsyncCodecLimits::new(512, 4, 16, 128, 256)
        .expect("reachable bounded fuzz codec profile");
    if let Ok(envelope) = decode_async_envelope(bytes, &limits, support::async_context()) {
        assert_eq!(
            encode_async_envelope(&envelope, &limits).expect("validated envelope re-encodes"),
            bytes,
        );
    }
});
