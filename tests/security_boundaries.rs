//! Redaction and telemetry-cardinality security boundary tests.

use suprnova_live::error::{ErrorCategory, RecoveryInstruction, SafeDiagnosticCode};
use suprnova_live::host::CheckKind;
use suprnova_live::identity::{ContentDigest, UnixMillis};
use suprnova_live::telemetry::{TelemetryEvent, TelemetryLabels, TelemetryOutcome};
use suprnova_live_test_support::{HarnessServices, HarnessTraceEvent};

#[test]
fn telemetry_labels_are_closed_and_never_accept_raw_payload_or_identity_strings() {
    let digest = ContentDigest::from_bytes(&[0x42; 32]).expect("digest is valid");
    let labels = TelemetryLabels::new(
        TelemetryEvent::SnapshotVerification,
        TelemetryOutcome::Rejected,
        ErrorCategory::Snapshot,
        RecoveryInstruction::RefreshIsland,
        SafeDiagnosticCode::SignatureInvalid,
        Some(&digest),
    );
    let encoded = labels.to_pairs();

    assert_eq!(encoded.len(), 6);
    assert!(
        encoded
            .iter()
            .all(|(key, value)| key.len() <= 32 && value.len() <= 32)
    );
    for forbidden in [
        "catalog.search",
        "search-results",
        "snapshot-v1",
        "secret-value",
    ] {
        assert!(!format!("{labels:?}{encoded:?}").contains(forbidden));
    }
}

#[test]
fn telemetry_event_and_outcome_cardinality_is_statically_bounded() {
    assert!(TelemetryEvent::ALL.len() <= 16);
    assert!(TelemetryOutcome::ALL.len() <= 8);
    assert!(
        TelemetryEvent::ALL
            .iter()
            .all(|value| value.as_str().len() <= 32)
    );
    assert!(
        TelemetryOutcome::ALL
            .iter()
            .all(|value| value.as_str().len() <= 32)
    );
}

#[test]
fn host_check_and_conformance_trace_cardinality_remain_closed_and_redacted() {
    assert_eq!(CheckKind::ALL.len(), 8);
    let services = HarnessServices::new(UnixMillis::new(1_000));
    services.trace().record(HarnessTraceEvent::Authorization);
    services.trace().record(HarnessTraceEvent::Validation);
    let diagnostic = format!("{services:?}{:?}", services.trace().events());
    for forbidden in ["cookie", "csrf-token", "session-secret", "browser-state"] {
        assert!(!diagnostic.contains(forbidden));
    }
}
