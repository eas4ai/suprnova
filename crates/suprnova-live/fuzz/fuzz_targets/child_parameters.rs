#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use suprnova_live::child::verify_child_parameters;
use suprnova_live::identity::UnixMillis;

fuzz_target!(|data: &[u8]| {
    let (Some(snapshot), Some(child)) = (support::snapshot_setup(), support::child_setup()) else {
        return;
    };
    let _ = verify_child_parameters(
        data,
        &child.expected,
        &snapshot.keys,
        UnixMillis::new(1_010),
        &child.limits,
    );
});
