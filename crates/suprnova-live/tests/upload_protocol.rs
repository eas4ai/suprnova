//! Bounded upload protocol-v1 codec contract tests.

use std::collections::BTreeSet;

use serde::Deserialize;
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    SUPPORTED_UPLOAD_PROTOCOL_VERSIONS, UploadOperation, UploadProtocolCodec,
};

#[derive(Deserialize)]
struct Fixture {
    protocol_versions: Vec<u16>,
    live_protocol_versions: Vec<u16>,
    operations: Vec<String>,
    codec_limits: FixtureLimits,
    codec_cases: Vec<CodecCase>,
}

#[derive(Deserialize)]
struct FixtureLimits {
    max_bytes: usize,
    max_depth: usize,
    max_entries: usize,
    max_string_bytes: usize,
}

#[derive(Deserialize)]
struct CodecCase {
    id: String,
    expected: String,
    encoded: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v4/upload-protocol.json"))
        .expect("upload fixture")
}

#[test]
fn locked_v4_codec_cases_decode_with_exact_dispositions() {
    let fixture = fixture();
    let codec = UploadProtocolCodec::new(
        fixture.codec_limits.max_bytes,
        fixture.codec_limits.max_depth,
        fixture.codec_limits.max_entries,
        fixture.codec_limits.max_string_bytes,
    )
    .expect("fixture limits");

    assert_eq!(
        fixture.protocol_versions,
        SUPPORTED_UPLOAD_PROTOCOL_VERSIONS
    );
    assert_eq!(fixture.live_protocol_versions, [1, 2]);

    let mut accepted = BTreeSet::new();
    for case in fixture.codec_cases {
        match codec.decode(case.encoded.as_bytes()) {
            Ok(operation) => {
                assert_eq!(case.expected, "accepted", "case {}", case.id);
                accepted.insert(operation.name().to_owned());
            }
            Err(error) => assert_eq!(error.kind().as_str(), case.expected, "case {}", case.id),
        }
    }

    assert_eq!(
        accepted,
        fixture.operations.into_iter().collect::<BTreeSet<_>>()
    );
}

#[test]
fn decoded_operations_are_typed_and_keep_wire_and_live_versions_independent() {
    let codec = UploadProtocolCodec::v1();

    let create = codec
        .decode(br#"{"expected_revision":"0","field":"avatar","idempotency_key":"create-01","operation":"create","protocol_version":1}"#)
        .expect("create");
    let UploadOperation::Create(create) = create else {
        panic!("expected create");
    };
    assert_eq!(create.expected_revision().get(), 0);
    assert_eq!(create.field().as_str(), "avatar");
    assert_eq!(create.idempotency_key().as_str(), "create-01");

    let chunk = codec
        .decode(br#"{"checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","chunk_index":0,"expected_revision":"2","handle":"018f47c1-2af0-7cc4-a001-000000000001","idempotency_key":"chunk-00","operation":"put_chunk","protocol_version":1,"size":262144}"#)
        .expect("chunk");
    let UploadOperation::PutChunk(chunk) = chunk else {
        panic!("expected chunk");
    };
    assert_eq!(
        chunk.handle().to_string(),
        "018f47c1-2af0-7cc4-a001-000000000001"
    );
    assert_eq!(chunk.chunk_index(), 0);
    assert_eq!(chunk.size(), 262_144);
    assert_eq!(chunk.checksum().as_str().len(), 64);
}

#[test]
fn codec_rejects_oversize_and_malformed_typed_fields_before_commands_exist() {
    let codec = UploadProtocolCodec::new(128, 4, 16, 64).expect("hostile limits");
    let oversized = format!(
        "{{\"operation\":\"status\",\"protocol_version\":1,\"handle\":\"{}\"}}",
        "x".repeat(128)
    );
    assert_eq!(
        codec
            .decode(oversized.as_bytes())
            .expect_err("oversize")
            .kind()
            .as_str(),
        "input_too_large"
    );

    let codec = UploadProtocolCodec::v1();
    for malformed in [
        br#"{"operation":"create","protocol_version":1,"expected_revision":"1","field":"avatar","idempotency_key":"create-01"}"#.as_slice(),
        br#"{"operation":"put_chunk","protocol_version":1,"handle":"018f47c1-2af0-7cc4-a001-000000000001","expected_revision":"2","idempotency_key":"chunk-00","chunk_index":-1,"size":262144,"checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#.as_slice(),
        br#"{"operation":"put_chunk","protocol_version":1,"handle":"018f47c1-2af0-7cc4-a001-000000000001","expected_revision":"2","idempotency_key":"chunk-00","chunk_index":0,"size":0,"checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#.as_slice(),
        br#"{"operation":"complete","protocol_version":1,"handle":"018f47c1-2af0-7cc4-a001-000000000001","expected_revision":"3","idempotency_key":"complete-01","whole_checksum":"NOT-A-CHECKSUM"}"#.as_slice(),
    ] {
        assert_eq!(
            codec.decode(malformed).expect_err("malformed field").kind().as_str(),
            "invalid_field"
        );
    }
}

#[test]
fn upload_limits_cover_every_amplification_dimension_and_reject_unbounded_profiles() {
    let config = UploadLimitConfig::reference();
    let limits = UploadLimits::new(config).expect("reference upload limits");

    assert!(limits.max_files_per_field() > 0);
    assert!(limits.max_pending_per_scope() >= limits.max_files_per_field());
    assert!(limits.max_file_bytes() >= limits.max_chunk_bytes() as u64);
    assert!(limits.max_chunks_per_file() > 0);
    assert!(limits.max_aggregate_bytes() >= limits.max_file_bytes());
    assert!(limits.max_in_flight_bytes() >= limits.max_chunk_bytes());
    assert!(limits.max_concurrent_transfers() > 0);
    assert!(limits.max_creations_per_window() > 0);
    assert!(limits.creation_window_ms() > 0);
    assert!(limits.max_retries() > 0);
    assert!(limits.max_age_ms() > 0);
    assert!(limits.max_validation_ms() > 0);
    assert!(limits.max_scan_ms() > 0);
    assert!(limits.max_storage_bytes() >= limits.max_aggregate_bytes());
    assert!(limits.max_cleanup_batch() > 0);
    assert!(limits.max_idempotency_outcomes() > 0);

    let mut zero = config;
    zero.max_chunk_bytes = 0;
    assert!(UploadLimits::new(zero).is_err());

    let mut zero_chunks = config;
    zero_chunks.max_chunks_per_file = 0;
    assert!(UploadLimits::new(zero_chunks).is_err());

    let mut unbounded = config;
    unbounded.max_pending_per_scope = usize::MAX;
    assert!(UploadLimits::new(unbounded).is_err());

    let mut inconsistent = config;
    inconsistent.max_chunk_bytes = inconsistent.max_file_bytes as usize + 1;
    assert!(UploadLimits::new(inconsistent).is_err());
}
