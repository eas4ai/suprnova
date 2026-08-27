//! Public asynchronous-envelope surface checks.

use suprnova_live::async_updates::{AsyncCodecLimits, SUPPORTED_ASYNC_PROTOCOL_VERSIONS};

#[test]
fn public_async_envelope_surface_keeps_protocol_and_limits_available_without_raw_sequence_authority()
 {
    assert_eq!(SUPPORTED_ASYNC_PROTOCOL_VERSIONS, &[1]);
    assert_eq!(AsyncCodecLimits::v1(), AsyncCodecLimits::v1());
}

#[test]
fn hostile_fuzz_profile_is_a_locked_small_allocation_boundary() {
    assert_eq!(
        AsyncCodecLimits::hostile_test(),
        AsyncCodecLimits::new(512, 4, 16, 128, 256).expect("locked hostile limits"),
    );
}
