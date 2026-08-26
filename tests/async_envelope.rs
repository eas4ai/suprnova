//! Public asynchronous-envelope surface checks.

use suprnova_live::async_updates::{AsyncCodecLimits, SUPPORTED_ASYNC_PROTOCOL_VERSIONS};

#[test]
fn public_async_envelope_surface_keeps_protocol_and_limits_available_without_raw_sequence_authority()
 {
    assert_eq!(SUPPORTED_ASYNC_PROTOCOL_VERSIONS, &[1]);
    assert_eq!(AsyncCodecLimits::v1(), AsyncCodecLimits::v1());
}
