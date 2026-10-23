#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::upload::MediaHeaderProbe;

const MAX_MEDIA_PREFIX_BYTES: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    let decoded = data
        .strip_prefix(b"hex:")
        .and_then(decode_hex)
        .unwrap_or_else(|| data[..data.len().min(MAX_MEDIA_PREFIX_BYTES)].to_vec());
    if let Ok(Some(dimensions)) = MediaHeaderProbe::probe(&decoded) {
        assert_ne!(dimensions.width(), 0);
        assert_ne!(dimensions.height(), 0);
        assert_eq!(
            dimensions.pixels(),
            u64::from(dimensions.width()) * u64::from(dimensions.height())
        );
    }
});

fn decode_hex(value: &[u8]) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || value.len() / 2 > MAX_MEDIA_PREFIX_BYTES {
        return None;
    }
    value
        .chunks_exact(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

const fn nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
