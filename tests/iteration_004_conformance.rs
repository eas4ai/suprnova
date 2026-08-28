//! One-process Rust/TypeScript parity over every v4 wire and transition case.

#[allow(
    dead_code,
    reason = "the shared transport fixture exposes a wider deterministic test surface"
)]
#[path = "support/async_transport.rs"]
mod async_transport;

use std::process::Command;

use serde::Deserialize;
use serde_json::{Value, json};
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncEnvelopeErrorKind, PresentationSignalContract, PresentationSignalSchema,
    StreamEpoch, StreamPosition, StreamSequence, SubscriptionId, decode_async_envelope,
};
use suprnova_live::identity::{SignalName, SignalScopeIdentity};
use suprnova_live::upload::{
    AcceptedChunk, TransitionDisposition, UploadChecksum, UploadHandle, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadRevision, UploadState, UploadStateMachine,
    UploadTransition, UploadTransitionRequest,
};

use async_transport::TransportFixture;

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const CHECKSUM: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[derive(Deserialize)]
struct UploadFixture {
    operations: Vec<String>,
    codec_cases: Vec<CodecCase>,
    transition_cases: Vec<TransitionCase>,
}

#[derive(Deserialize)]
struct AsyncFixture {
    envelope_cases: Vec<CodecCase>,
    continuity_cases: Vec<ContinuityCase>,
}

#[derive(Deserialize)]
struct CodecCase {
    id: String,
    encoded: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InternalUploadTransition {
    Queue,
    BeginTransfer,
    PutChunk,
    Complete,
    Accept,
    BeginFinalize,
    CommitFinalize,
    Cancel,
    Reject,
    Expire,
}

#[derive(Deserialize)]
struct TransitionCase {
    id: String,
    from: String,
    operation: InternalUploadTransition,
    chunk_index: Option<u32>,
    idempotency_key: Option<String>,
    expected_revision: String,
    to: String,
    next_revision: String,
    expected: String,
}

#[derive(Deserialize)]
struct ContinuityCase {
    id: String,
    baseline: FixturePosition,
    observed: Option<FixturePosition>,
    observed_gap: Option<FixturePosition>,
    recovery: Option<Recovery>,
}

#[derive(Clone, Copy, Deserialize)]
struct FixturePosition {
    #[serde(deserialize_with = "decimal_u64")]
    epoch: u64,
    #[serde(deserialize_with = "decimal_u64")]
    sequence: u64,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Recovery {
    Replay { transcript: Vec<FixturePosition> },
    AuthoritativeRefresh { baseline: FixturePosition },
}

fn decimal_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    encoded.parse().map_err(serde::de::Error::custom)
}

#[tokio::test]
async fn every_v4_case_has_identical_rust_and_typescript_canonical_outcomes() {
    let upload: UploadFixture =
        serde_json::from_str(include_str!("../fixtures/v4/upload-protocol.json"))
            .expect("upload fixture");
    let asynchronous: AsyncFixture =
        serde_json::from_str(include_str!("../fixtures/v4/async-envelope.json"))
            .expect("async fixture");

    assert_eq!(
        upload.operations,
        [
            "create",
            "put_chunk",
            "status",
            "complete",
            "cancel",
            "reacquire"
        ]
    );

    let transport = TransportFixture::new(stream_position(FixturePosition {
        epoch: 4,
        sequence: 40,
    }))
    .await;
    transport
        .registry
        .set_presentation_signals(vec![PresentationSignalContract::new(
            SignalScopeIdentity::parse("root-scope").expect("signal scope"),
            SignalName::parse("completion_percent").expect("signal name"),
            PresentationSignalSchema::U64,
        )]);
    let authorization = transport.request(
        SubscriptionId::from_bytes(b"subscription-001").expect("fixture subscription"),
        suprnova_live::async_updates::VerifiedOrigin::parse("https://app.example.test")
            .expect("fixture origin"),
    );

    let rust = json!({
        "async_continuity": asynchronous
            .continuity_cases
            .iter()
            .map(rust_continuity_case)
            .collect::<Vec<_>>(),
        "async_envelopes": asynchronous
            .envelope_cases
            .iter()
            .map(|case| rust_async_case(case, authorization.context()))
            .collect::<Vec<_>>(),
        "upload_codecs": upload
            .codec_cases
            .iter()
            .map(rust_upload_codec_case)
            .collect::<Vec<_>>(),
        "upload_transitions": upload
            .transition_cases
            .iter()
            .map(rust_transition_case)
            .collect::<Vec<_>>(),
    });
    let typescript = typescript_report();

    assert_eq!(rust, typescript);
}

fn rust_upload_codec_case(case: &CodecCase) -> Value {
    match UploadProtocolCodec::v1().decode(case.encoded.as_bytes()) {
        Ok(operation) => {
            let _: &'static str = match operation {
                UploadOperation::Create(_) => "create",
                UploadOperation::PutChunk(_) => "put_chunk",
                UploadOperation::Status(_) => "status",
                UploadOperation::Complete(_) => "complete",
                UploadOperation::Cancel(_) => "cancel",
                UploadOperation::Reacquire(_) => "reacquire",
            };
            json!({"code": null, "disposition": "accepted", "id": case.id})
        }
        Err(error) => json!({
            "code": error.kind().as_str(),
            "disposition": "rejected",
            "id": case.id,
        }),
    }
}

fn rust_transition_case(case: &TransitionCase) -> Value {
    let expected_revision = UploadRevision::parse(&case.expected_revision).expect("revision");
    let next_revision = UploadRevision::parse(&case.next_revision).expect("next revision");
    let from = UploadState::parse(&case.from).expect("from state");
    let request = transition_request(case);
    let (code, disposition, state, position) = match case.expected.as_str() {
        "applied" => {
            let mut machine = UploadStateMachine::new(upload_handle(), from, expected_revision);
            let outcome = machine.apply(request).expect("applied fixture transition");
            assert_eq!(outcome.disposition(), TransitionDisposition::Applied);
            (
                None,
                "applied",
                outcome.state().as_str(),
                outcome.revision().get().to_string(),
            )
        }
        "existing_outcome" => {
            let mut machine = UploadStateMachine::new(upload_handle(), from, expected_revision);
            machine
                .apply(request.clone())
                .expect("initial fixture transition");
            let outcome = machine.apply(request).expect("idempotent fixture replay");
            assert_eq!(
                outcome.disposition(),
                TransitionDisposition::ExistingOutcome
            );
            (
                None,
                "existing_outcome",
                outcome.state().as_str(),
                outcome.revision().get().to_string(),
            )
        }
        "conflict" => {
            let mut machine = UploadStateMachine::new(upload_handle(), from, next_revision);
            let error = machine.apply(request).expect_err("fixture conflict");
            (
                Some(error.kind().as_str()),
                "conflict",
                machine.state().as_str(),
                machine.revision().get().to_string(),
            )
        }
        other => panic!("unknown transition disposition {other}"),
    };
    assert_eq!(state, case.to);
    assert_eq!(position, case.next_revision);
    json!({
        "code": code,
        "disposition": disposition,
        "id": case.id,
        "position": position,
        "state": state,
    })
}

fn transition_request(case: &TransitionCase) -> UploadTransitionRequest {
    let transition = match case.operation {
        InternalUploadTransition::Queue => UploadTransition::Queue,
        InternalUploadTransition::BeginTransfer => UploadTransition::BeginTransfer,
        InternalUploadTransition::PutChunk => UploadTransition::PutChunk(
            AcceptedChunk::new(
                case.chunk_index.expect("chunk index"),
                262_144,
                UploadChecksum::parse(CHECKSUM).expect("checksum"),
            )
            .expect("accepted chunk"),
        ),
        InternalUploadTransition::Complete => UploadTransition::Complete,
        InternalUploadTransition::Accept => UploadTransition::Accept,
        InternalUploadTransition::BeginFinalize => UploadTransition::BeginFinalize,
        InternalUploadTransition::CommitFinalize => UploadTransition::CommitFinalize,
        InternalUploadTransition::Cancel => UploadTransition::Cancel,
        InternalUploadTransition::Reject => UploadTransition::Reject,
        InternalUploadTransition::Expire => UploadTransition::Expire,
    };
    UploadTransitionRequest::new(
        upload_handle(),
        UploadRevision::parse(&case.expected_revision).expect("expected revision"),
        UploadIdempotencyKey::parse(case.idempotency_key.as_deref().unwrap_or(&case.id))
            .expect("idempotency key"),
        transition,
    )
}

fn rust_async_case(
    case: &CodecCase,
    context: &suprnova_live::async_updates::AsyncEnvelopeContext,
) -> Value {
    match decode_async_envelope(case.encoded.as_bytes(), &AsyncCodecLimits::v1(), context) {
        Ok(envelope) => json!({
            "code": null,
            "disposition": "accepted",
            "id": case.id,
            "position": encoded_position(envelope.position()),
        }),
        Err(error) => json!({
            "code": canonical_async_code(error.kind()),
            "disposition": "rejected",
            "id": case.id,
            "position": null,
        }),
    }
}

const fn canonical_async_code(kind: AsyncEnvelopeErrorKind) -> &'static str {
    match kind {
        AsyncEnvelopeErrorKind::UnsupportedProtocol => "unsupported_protocol",
        AsyncEnvelopeErrorKind::DuplicateField => "duplicate_field",
        AsyncEnvelopeErrorKind::UnsupportedPayload => "unsupported_payload",
        _ => kind.as_str(),
    }
}

fn rust_continuity_case(case: &ContinuityCase) -> Value {
    let baseline = stream_position(case.baseline);
    let (disposition, state, position) = if let Some(observed) = case.observed {
        let observed = stream_position(observed);
        if observed == baseline {
            ("ignore_duplicate", "current", baseline)
        } else if observed.epoch() == baseline.epoch()
            && observed.sequence().get() == baseline.sequence().get() + 1
        {
            ("apply", "current", observed)
        } else {
            ("degrade", "degraded", baseline)
        }
    } else {
        let observed = stream_position(case.observed_gap.expect("observed gap"));
        assert!(
            observed.epoch() > baseline.epoch()
                || observed.sequence().get() > baseline.sequence().get() + 1
        );
        let recovered = match case.recovery.as_ref().expect("recovery") {
            Recovery::Replay { transcript } => {
                let mut prior = baseline;
                for position in transcript {
                    let current = stream_position(*position);
                    assert_eq!(current.epoch(), prior.epoch());
                    assert_eq!(current.sequence().get(), prior.sequence().get() + 1);
                    prior = current;
                }
                prior
            }
            Recovery::AuthoritativeRefresh { baseline } => stream_position(*baseline),
        };
        ("adopt_baseline", "current", recovered)
    };
    json!({
        "disposition": disposition,
        "id": case.id,
        "position": encoded_position(position),
        "state": state,
    })
}

fn stream_position(value: FixturePosition) -> StreamPosition {
    StreamPosition::new(
        StreamEpoch::new(value.epoch),
        StreamSequence::new(value.sequence),
    )
}

fn encoded_position(value: StreamPosition) -> Value {
    json!({
        "epoch": value.epoch().get().to_string(),
        "sequence": value.sequence().get().to_string(),
    })
}

fn upload_handle() -> UploadHandle {
    UploadHandle::parse(HANDLE).expect("fixture upload handle")
}

fn typescript_report() -> Value {
    let output = Command::new("node")
        .args([
            "--no-warnings=ExperimentalWarning",
            "--experimental-transform-types",
            "--loader",
            "./tests/support/typescript-loader.mjs",
            "./browser/tests/support/iteration-004-conformance.ts",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run TypeScript conformance parser");
    assert!(
        output.status.success(),
        "TypeScript conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("TypeScript conformance JSON")
}
