#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;
use suprnova_live::identity::UnixMillis;
use suprnova_live::snapshot::{verify_instance, verify_seed};

fuzz_target!(|data: &[u8]| {
    let Some(setup) = support::snapshot_setup() else {
        return;
    };
    let _ = verify_seed(
        data,
        &setup.seed,
        &setup.keys,
        UnixMillis::new(1_010),
        &setup.limits,
    );
    let _ = verify_instance(
        data,
        &setup.instance,
        &setup.keys,
        UnixMillis::new(1_010),
        &setup.limits,
    );
});
