//! One-process Rust/TypeScript parity over every v4 wire and transition case.

#[allow(
    dead_code,
    reason = "the shared transport fixture exposes a wider deterministic test surface"
)]
#[path = "support/async_transport.rs"]
mod async_transport;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::process::Command;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncEnvelopeErrorKind, PresentationSignalContract, PresentationSignalSchema,
    SequenceConformanceMachine, SequenceDisposition, SequenceState, StreamEpoch, StreamPosition,
    StreamSequence, SubscriptionId, decode_async_envelope,
};
use suprnova_live::conformance::{FIXTURE_FILES_V4, FixtureVersion, fixture_directory};
use suprnova_live::error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};
use suprnova_live::identity::{SignalName, SignalScopeIdentity};
use suprnova_live::resource::{Permit, PermitPool, ResourceBounds, ResourceOwner};
use suprnova_live::upload::{
    AcceptedChunk, TransitionDisposition, UploadChecksum, UploadHandle, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadRevision, UploadState, UploadStateMachine,
    UploadTransition, UploadTransitionRequest,
};

use async_transport::TransportFixture;

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const CHECKSUM: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn actual_matches_fixture_oracle(actual: &Value, expected: &Value) -> Result<(), String> {
    match expected {
        Value::Object(expected) => {
            let actual = actual
                .as_object()
                .ok_or_else(|| format!("expected object oracle, received {actual}"))?;
            for (field, expected) in expected {
                let actual = actual
                    .get(field)
                    .ok_or_else(|| format!("missing oracle field {field}"))?;
                actual_matches_fixture_oracle(actual, expected)
                    .map_err(|error| format!("{field}: {error}"))?;
            }
            Ok(())
        }
        Value::Array(expected) => {
            let actual = actual
                .as_array()
                .ok_or_else(|| format!("expected array oracle, received {actual}"))?;
            if actual.len() != expected.len() {
                return Err(format!(
                    "oracle array length {} != actual {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                actual_matches_fixture_oracle(actual, expected)
                    .map_err(|error| format!("[{index}]: {error}"))?;
            }
            Ok(())
        }
        _ if actual == expected => Ok(()),
        _ => Err(format!("fixture oracle {expected} != actual {actual}")),
    }
}

#[test]
fn wrong_fixture_expected_fails_even_when_rust_and_typescript_agree() {
    let agreed_actual = json!({
        "code": null,
        "disposition": "applied",
        "position": "2",
        "state": "queued",
    });
    let mutated_fixture_oracle = json!({
        "disposition": "rejected",
        "position": "2",
        "state": "queued",
    });

    assert!(actual_matches_fixture_oracle(&agreed_actual, &mutated_fixture_oracle).is_err());
}

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
    signal_name_cases: Vec<SignalNameCase>,
}

#[derive(Deserialize)]
struct SignalNameCase {
    value: String,
    expected: String,
}

#[derive(Deserialize)]
struct CompatibilityFixture {
    cases: Vec<CompatibilityCase>,
}

#[derive(Deserialize)]
struct CompatibilityCase {
    id: String,
    feature: String,
    present: bool,
    capability_version: Option<u16>,
    core_version: String,
    expected: String,
}

#[derive(Deserialize)]
struct RuntimeFeatureFixture {
    features: Vec<RuntimeFeatureContract>,
}

#[derive(Deserialize)]
struct RuntimeFeatureContract {
    name: String,
    capability_version: u16,
    compatible_core: CoreCompatibility,
}

#[derive(Deserialize)]
struct CoreCompatibility {
    minimum: String,
    maximum_exclusive: String,
}

#[derive(Deserialize)]
struct DiagnosticFixture {
    redaction_cases: Vec<RedactionCase>,
}

#[derive(Deserialize)]
struct RedactionCase {
    id: String,
    sample: Value,
    expected: String,
}

#[derive(Deserialize)]
struct ResourceFixture {
    bounds: ResourceFixtureBounds,
    cases: Vec<ResourceCase>,
}

#[derive(Clone, Copy, Deserialize)]
struct ResourceFixtureBounds {
    max_items: usize,
    max_bytes: usize,
    max_active: usize,
}

#[derive(Deserialize)]
struct ResourceCase {
    id: String,
    operations: Vec<ResourceOperation>,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum ResourceOperation {
    Enqueue { bytes: usize, expected: Value },
    Acquire { expected: Value },
    Release { expected: Value },
    Retire { expected: Value },
    Resume { expected: Value },
    Suspend { expected: Value },
}

impl ResourceOperation {
    fn expected(&self) -> &Value {
        match self {
            Self::Enqueue { expected, .. }
            | Self::Acquire { expected }
            | Self::Release { expected }
            | Self::Retire { expected }
            | Self::Resume { expected }
            | Self::Suspend { expected } => expected,
        }
    }
}

#[derive(Deserialize)]
struct CodecCase {
    id: String,
    encoded: String,
    expected: String,
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
    Fail,
}

#[derive(Deserialize)]
struct TransitionCase {
    id: String,
    from: String,
    operation: InternalUploadTransition,
    chunk_index: Option<u32>,
    idempotency_key: Option<String>,
    expected_revision: String,
    current_revision: Option<String>,
    retry: Option<Value>,
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
    expected: String,
    state: String,
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

fn load_v4_fixture<T: DeserializeOwned>(name: &str) -> T {
    serde_json::from_slice(
        &fs::read(fixture_directory(FixtureVersion::V4).join(name))
            .unwrap_or_else(|error| panic!("read v4 fixture {name}: {error}")),
    )
    .unwrap_or_else(|error| panic!("parse v4 fixture {name}: {error}"))
}

fn v4_inventory() -> Value {
    let directory = fixture_directory(FixtureVersion::V4);
    let actual_files = fs::read_dir(&directory)
        .expect("read v4 fixture directory")
        .map(|entry| entry.expect("v4 fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            path.file_name()
                .expect("v4 fixture file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_files,
        FIXTURE_FILES_V4
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        "new v4 fixture files must be explicitly added to cross-language parity"
    );

    let mut actual = BTreeMap::new();
    for name in FIXTURE_FILES_V4 {
        let fixture: Value = load_v4_fixture(name);
        let mut keys = fixture
            .as_object()
            .unwrap_or_else(|| panic!("v4 fixture {name} is an object"))
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        actual.insert((*name).to_owned(), keys);
    }
    let expected = BTreeMap::from([
        (
            "async-envelope.json".to_owned(),
            vec![
                "codec_limits",
                "continuity_cases",
                "envelope_cases",
                "live_protocol_versions",
                "payload_kinds",
                "protocol_versions",
                "schema_version",
                "signal_name_cases",
                "subscription_states",
            ],
        ),
        (
            "compatibility.json".to_owned(),
            vec![
                "cases",
                "compatible_core",
                "live_protocol_versions",
                "schema_version",
                "snapshot_versions",
            ],
        ),
        (
            "diagnostics.json".to_owned(),
            vec![
                "allowed_dimensions",
                "codes",
                "phases",
                "redacted_classes",
                "redaction_cases",
                "retention",
                "schema_version",
                "severities",
            ],
        ),
        (
            "directive-grammar.json".to_owned(),
            vec![
                "contract_version",
                "directives",
                "event_modifiers",
                "feedback_modifiers",
                "freshness_combinations",
                "model_modifiers",
                "morph_modifiers",
                "navigation_modifiers",
                "reserved",
                "schema_version",
                "syntax",
                "transition_modifiers",
            ],
        ),
        (
            "resource-lifecycle.json".to_owned(),
            vec![
                "bounds",
                "cases",
                "resource_kinds",
                "schema_version",
                "states",
            ],
        ),
        (
            "runtime-features.json".to_owned(),
            vec![
                "allowed_island_operations",
                "features",
                "forbidden_island_operations",
                "registration_outcomes",
                "registry",
                "retirement",
                "schema_version",
            ],
        ),
        (
            "upload-protocol.json".to_owned(),
            vec![
                "codec_cases",
                "codec_limits",
                "live_protocol_versions",
                "operations",
                "presentation_states",
                "protocol_versions",
                "schema_version",
                "states",
                "terminal_states",
                "transition_cases",
            ],
        ),
    ])
    .into_iter()
    .map(|(name, keys)| {
        (
            name,
            keys.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    })
    .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual, expected,
        "new v4 fixture collections must be explicitly added to parity"
    );
    json!(actual)
}

#[tokio::test]
async fn every_v4_case_has_identical_rust_and_typescript_canonical_outcomes() {
    let upload: UploadFixture = load_v4_fixture("upload-protocol.json");
    let asynchronous: AsyncFixture = load_v4_fixture("async-envelope.json");
    let compatibility: CompatibilityFixture = load_v4_fixture("compatibility.json");
    let diagnostics: DiagnosticFixture = load_v4_fixture("diagnostics.json");
    let resources: ResourceFixture = load_v4_fixture("resource-lifecycle.json");
    let runtime_features: RuntimeFeatureFixture = load_v4_fixture("runtime-features.json");
    let _: Value = load_v4_fixture("directive-grammar.json");
    let inventory = v4_inventory();

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
            .map(|case| rust_continuity_case(case, authorization.context()))
            .collect::<Vec<_>>(),
        "async_envelopes": asynchronous
            .envelope_cases
            .iter()
            .map(|case| rust_async_case(case, authorization.context()))
            .collect::<Vec<_>>(),
        "async_signals": asynchronous
            .signal_name_cases
            .iter()
            .map(rust_signal_name_case)
            .collect::<Vec<_>>(),
        "compatibility": compatibility
            .cases
            .iter()
            .map(|case| rust_compatibility_case(case, &runtime_features.features))
            .collect::<Vec<_>>(),
        "diagnostics": diagnostics
            .redaction_cases
            .iter()
            .map(rust_redaction_case)
            .collect::<Vec<_>>(),
        "inventory": inventory,
        "resource_lifecycle": resources
            .cases
            .iter()
            .flat_map(|case| rust_resource_case(case, resources.bounds))
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

    assert_v4_fixture_oracles(
        "Rust",
        &rust,
        &upload,
        &asynchronous,
        &compatibility,
        &diagnostics,
        &resources,
    );
    assert_v4_fixture_oracles(
        "TypeScript",
        &typescript,
        &upload,
        &asynchronous,
        &compatibility,
        &diagnostics,
        &resources,
    );

    let report_collections = typescript
        .as_object()
        .expect("TypeScript report object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        report_collections,
        [
            "async_continuity",
            "async_envelopes",
            "async_signals",
            "compatibility",
            "diagnostics",
            "inventory",
            "resource_lifecycle",
            "upload_codecs",
            "upload_transitions",
        ]
        .into_iter()
        .collect(),
        "every case-bearing v4 collection must participate in the one cross-language report"
    );

    assert_eq!(rust, typescript);
}

fn assert_collection_oracles(
    implementation: &str,
    report: &Value,
    collection: &str,
    expected: Vec<Value>,
) {
    let actual = report
        .get(collection)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{implementation} report omits {collection}"));
    assert_eq!(
        actual.len(),
        expected.len(),
        "{implementation} {collection} case count differs from fixture oracle"
    );
    for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        actual_matches_fixture_oracle(actual, expected).unwrap_or_else(|error| {
            panic!("{implementation} {collection}[{index}] violates fixture oracle: {error}")
        });
    }
}

fn assert_v4_fixture_oracles(
    implementation: &str,
    report: &Value,
    upload: &UploadFixture,
    asynchronous: &AsyncFixture,
    compatibility: &CompatibilityFixture,
    diagnostics: &DiagnosticFixture,
    resources: &ResourceFixture,
) {
    assert_collection_oracles(
        implementation,
        report,
        "upload_codecs",
        upload
            .codec_cases
            .iter()
            .map(|case| {
                json!({
                    "code": (case.expected != "accepted").then_some(case.expected.as_str()),
                    "disposition": if case.expected == "accepted" { "accepted" } else { "rejected" },
                    "id": case.id,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "upload_transitions",
        upload
            .transition_cases
            .iter()
            .map(|case| {
                json!({
                    "code": (case.expected == "conflict").then_some("upload_conflict"),
                    "disposition": case.expected,
                    "id": case.id,
                    "position": case.next_revision,
                    "state": case.to,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "async_envelopes",
        asynchronous
            .envelope_cases
            .iter()
            .map(|case| {
                json!({
                    "code": (case.expected != "accepted").then_some(case.expected.as_str()),
                    "disposition": if case.expected == "accepted" { "accepted" } else { "rejected" },
                    "id": case.id,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "async_signals",
        asynchronous
            .signal_name_cases
            .iter()
            .map(|case| {
                json!({
                    "code": (case.expected != "accepted").then_some("invalid_signal_name"),
                    "disposition": case.expected,
                    "id": case.value,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "async_continuity",
        asynchronous
            .continuity_cases
            .iter()
            .map(|case| {
                json!({
                    "disposition": case.expected,
                    "id": case.id,
                    "state": case.state,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "compatibility",
        compatibility
            .cases
            .iter()
            .map(|case| {
                json!({
                    "code": (case.expected == "feature_unavailable").then_some("feature_unavailable"),
                    "disposition": case.expected,
                    "id": case.id,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "diagnostics",
        diagnostics
            .redaction_cases
            .iter()
            .map(|case| {
                json!({
                    "code": null,
                    "disposition": "redacted",
                    "id": case.id,
                    "state": case.expected,
                })
            })
            .collect(),
    );
    assert_collection_oracles(
        implementation,
        report,
        "resource_lifecycle",
        resources
            .cases
            .iter()
            .flat_map(|case| {
                case.operations
                    .iter()
                    .enumerate()
                    .map(|(index, operation)| {
                        json!({
                            "id": format!("{}:{index}", case.id),
                            "outcome": operation.expected(),
                        })
                    })
            })
            .collect(),
    );
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
    let from = UploadState::parse(&case.from).expect("from state");
    let request = transition_request(case);
    let current_revision = case
        .current_revision
        .as_deref()
        .map_or(expected_revision, |value| {
            UploadRevision::parse(value).expect("current revision")
        });
    let mut machine = UploadStateMachine::new(upload_handle(), from, current_revision);
    let result = if case.retry.is_some() {
        machine = UploadStateMachine::new(upload_handle(), from, expected_revision);
        machine
            .apply(request.clone())
            .expect("initial fixture transition");
        machine.apply(request)
    } else {
        machine.apply(request)
    };
    let (code, disposition, state, position) = match result {
        Ok(outcome) => (
            None,
            match outcome.disposition() {
                TransitionDisposition::Applied => "applied",
                TransitionDisposition::ExistingOutcome => "existing_outcome",
            },
            outcome.state().as_str(),
            outcome.revision().get().to_string(),
        ),
        Err(error) => (
            Some(error.kind().as_str()),
            if error.kind().as_str() == "upload_conflict" {
                "conflict"
            } else {
                "rejected"
            },
            machine.state().as_str(),
            machine.revision().get().to_string(),
        ),
    };
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
        InternalUploadTransition::Fail => UploadTransition::Fail,
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

fn rust_signal_name_case(case: &SignalNameCase) -> Value {
    match SignalName::parse(&case.value) {
        Ok(_) => json!({"code": null, "disposition": "accepted", "id": case.value}),
        Err(_) => json!({
            "code": "invalid_signal_name",
            "disposition": "rejected",
            "id": case.value,
        }),
    }
}

fn rust_compatibility_case(
    case: &CompatibilityCase,
    contracts: &[RuntimeFeatureContract],
) -> Value {
    if !case.present {
        return json!({
            "code": null,
            "disposition": "ordinary_live_available",
            "id": case.id,
        });
    }
    let Some(contract) = contracts
        .iter()
        .find(|contract| contract.name == case.feature)
    else {
        return json!({
            "code": "feature_unavailable",
            "disposition": "feature_unavailable",
            "id": case.id,
        });
    };
    let core_compatible = compare_version(&case.core_version, &contract.compatible_core.minimum)
        .is_ge()
        && compare_version(
            &case.core_version,
            &contract.compatible_core.maximum_exclusive,
        )
        .is_lt();
    let compatible =
        core_compatible && case.capability_version == Some(contract.capability_version);
    json!({
        "code": if compatible { None } else { Some("feature_unavailable") },
        "disposition": if compatible { "compatible" } else { "feature_unavailable" },
        "id": case.id,
    })
}

fn compare_version(left: &str, right: &str) -> std::cmp::Ordering {
    version(left).cmp(&version(right))
}

fn version(value: &str) -> [u64; 3] {
    let mut segments = value.split('.');
    let parsed = [
        segments.next().and_then(|value| value.parse().ok()),
        segments.next().and_then(|value| value.parse().ok()),
        segments.next().and_then(|value| value.parse().ok()),
    ];
    assert!(segments.next().is_none(), "invalid fixture semver {value}");
    parsed.map(|segment| segment.unwrap_or_else(|| panic!("invalid fixture semver {value}")))
}

#[derive(Debug)]
struct UnsafeDiagnosticSource(String);

impl fmt::Display for UnsafeDiagnosticSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for UnsafeDiagnosticSource {}

fn rust_redaction_case(case: &RedactionCase) -> Value {
    let unsafe_value = case.sample.as_str().map_or_else(
        || serde_json::to_string(&case.sample).expect("diagnostic sample JSON"),
        str::to_owned,
    );
    let error = LiveError::new(
        ErrorCategory::Security,
        RecoveryInstruction::Stop,
        SafeDiagnosticCode::InvalidIdentifier,
    )
    .with_source(UnsafeDiagnosticSource(unsafe_value.clone()));
    let rendered = format!("{error:?}");
    let redacted = !rendered.contains(&unsafe_value);
    json!({
        "code": if redacted { None } else { Some("diagnostic_value_leaked") },
        "disposition": if redacted { "redacted" } else { "rejected" },
        "id": case.id,
        "state": if redacted { Some("[redacted]") } else { None },
    })
}

#[derive(Clone, Copy)]
enum ResourceLifecycleState {
    Active,
    Suspended,
    Retired,
}

impl ResourceLifecycleState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
        }
    }
}

fn rust_resource_case(case: &ResourceCase, bounds: ResourceFixtureBounds) -> Vec<Value> {
    let owner = ResourceOwner::new(
        ResourceBounds::new(bounds.max_items, bounds.max_bytes).expect("resource bounds"),
    );
    let pool = PermitPool::new(bounds.max_active).expect("permit bounds");
    let mut permit: Option<Permit> = None;
    let mut state = ResourceLifecycleState::Active;
    case.operations
        .iter()
        .enumerate()
        .map(|(index, operation)| {
            let outcome = match operation {
                ResourceOperation::Enqueue { bytes, .. } => owner
                    .queue()
                    .try_push(*bytes, format!("item-{index}"))
                    .map_or_else(|error| json!(error.as_str()), |()| json!("accepted")),
                ResourceOperation::Acquire { .. } => match state {
                    ResourceLifecycleState::Suspended => json!("suspended"),
                    ResourceLifecycleState::Retired => json!("retired"),
                    ResourceLifecycleState::Active => match pool.try_acquire() {
                        Ok(acquired) => {
                            permit = Some(acquired);
                            json!("acquired")
                        }
                        Err(error) => json!(error.as_str()),
                    },
                },
                ResourceOperation::Release { .. } => {
                    permit.take().expect("fixture permit").release();
                    json!("released")
                }
                ResourceOperation::Retire { .. } => {
                    let released_permits = pool.active();
                    drop(permit.take());
                    let retirement = owner.retire();
                    state = ResourceLifecycleState::Retired;
                    json!({
                        "canceled": retirement.canceled,
                        "drained_bytes": retirement.drained_bytes,
                        "drained_items": retirement.drained_items,
                        "released_permits": released_permits,
                    })
                }
                ResourceOperation::Resume { .. } => {
                    if matches!(state, ResourceLifecycleState::Suspended) {
                        state = ResourceLifecycleState::Active;
                    }
                    json!(state.as_str())
                }
                ResourceOperation::Suspend { .. } => {
                    if matches!(state, ResourceLifecycleState::Active) {
                        state = ResourceLifecycleState::Suspended;
                    }
                    json!(state.as_str())
                }
            };
            json!({
                "code": null,
                "disposition": outcome.as_str().unwrap_or("retired"),
                "id": format!("{}:{index}", case.id),
                "outcome": outcome,
                "position": index.to_string(),
                "state": state.as_str(),
            })
        })
        .collect()
}

fn rust_continuity_case(
    case: &ContinuityCase,
    context: &suprnova_live::async_updates::AsyncEnvelopeContext,
) -> Value {
    let mut machine = SequenceConformanceMachine::new(context, stream_position(case.baseline))
        .expect("fixture continuity baseline");
    let disposition = if let Some(observed) = case.observed {
        match machine.observe(stream_position(observed)) {
            SequenceDisposition::Apply => "apply",
            SequenceDisposition::IgnoreDuplicate => "ignore_duplicate",
            SequenceDisposition::Degraded(_) => "degrade",
            other => panic!("unexpected fixture observation {other:?}"),
        }
    } else {
        let observed = stream_position(case.observed_gap.expect("observed gap"));
        assert!(matches!(
            machine.observe(observed),
            SequenceDisposition::Degraded(_)
        ));
        match case.recovery.as_ref().expect("recovery") {
            Recovery::Replay { transcript } => {
                let transcript = transcript
                    .iter()
                    .copied()
                    .map(stream_position)
                    .collect::<Vec<_>>();
                machine
                    .recover_from_replay(&transcript)
                    .expect("fixture replay recovery");
            }
            Recovery::AuthoritativeRefresh { baseline } => {
                machine
                    .install_authoritative_baseline(stream_position(*baseline))
                    .expect("fixture authoritative recovery");
            }
        }
        "adopt_baseline"
    };
    json!({
        "disposition": disposition,
        "id": case.id,
        "position": encoded_position(machine.current()),
        "state": match machine.state() {
            SequenceState::Current => "current",
            SequenceState::Degraded => "degraded",
        },
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

#[test]
fn typescript_conformance_runner_is_a_plain_supported_node_bundle() {
    let runner = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("browser/generated/iteration-004-conformance.mjs");
    let source = fs::read_to_string(&runner).expect("generated plain-Node conformance runner");
    assert!(!source.contains("experimental-transform-types"));
    assert!(!source.contains("typescript-loader"));

    let output = Command::new("node")
        .arg(&runner)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run plain-Node conformance parser");
    assert!(
        output.status.success(),
        "plain-Node conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _: Value = serde_json::from_slice(&output.stdout).expect("plain-Node conformance JSON");
}

fn typescript_report() -> Value {
    let output = Command::new("node")
        .arg("./browser/generated/iteration-004-conformance.mjs")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run generated TypeScript conformance parser");
    assert!(
        output.status.success(),
        "TypeScript conformance failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("TypeScript conformance JSON")
}
