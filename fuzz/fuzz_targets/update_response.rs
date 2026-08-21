#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use suprnova_live::protocol::parse_update_response;

fuzz_target!(|data: &[u8]| {
    let Some(limits) = support::protocol_limits() else {
        return;
    };
    let _ = parse_update_response(data, &limits);
});
