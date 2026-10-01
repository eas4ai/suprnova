#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::canonical::parse_canonical_value;
use suprnova_live::limits::InputLimits;

fuzz_target!(|data: &[u8]| {
    let Ok(limits) = InputLimits::new(2_048, 8, 128, 512) else {
        return;
    };
    let _ = parse_canonical_value(data, &limits);
});
