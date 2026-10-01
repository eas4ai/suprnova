#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::upload::UploadProtocolCodec;

const MAX_PROTOCOL_BYTES: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    let codec = UploadProtocolCodec::new(MAX_PROTOCOL_BYTES, 16, 128, 8 * 1024)
        .expect("fixed hostile-test upload limits are valid");
    if let Ok(operation) = codec.decode(data) {
        assert!(matches!(
            operation.name(),
            "create" | "put_chunk" | "status" | "complete" | "cancel" | "reacquire"
        ));
    }
});
