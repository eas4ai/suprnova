//! Redaction and telemetry-cardinality security boundary tests.

use suprnova_live::error::{ErrorCategory, RecoveryInstruction, SafeDiagnosticCode};
use suprnova_live::host::CheckKind;
use suprnova_live::identity::{ContentDigest, UnixMillis};
use suprnova_live::telemetry::{TelemetryEvent, TelemetryLabels, TelemetryOutcome};
use suprnova_live_test_support::{HarnessServices, HarnessTraceEvent};

fn repository_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn browser_sources() -> String {
    fn collect(directory: &std::path::Path, output: &mut String) {
        for entry in std::fs::read_dir(directory).expect("browser source directory") {
            let path = entry.expect("browser source entry").path();
            if path.is_dir() {
                collect(&path, output);
            } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("ts") {
                output.push_str(&std::fs::read_to_string(path).expect("UTF-8 browser source"));
            }
        }
    }
    let mut source = String::new();
    collect(&repository_root().join("browser/src"), &mut source);
    source
}

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

#[test]
fn browser_production_boundaries_forbid_dynamic_execution_and_client_authority() {
    let source = browser_sources();
    for forbidden in [
        "eval(",
        "new Function(",
        "crypto.subtle.sign",
        "createHmac(",
        "data-suprnova-live-snapshot=",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden browser source: {forbidden}"
        );
    }

    let metadata =
        std::fs::read_to_string(repository_root().join("browser/src/islands/metadata.ts"))
            .expect("metadata source");
    for required_bound in [
        "MAX_METADATA_ATTRIBUTES",
        "MAX_METADATA_UNITS",
        "MAX_IDENTITY_UNITS",
    ] {
        assert!(
            metadata.contains(required_bound),
            "missing metadata bound: {required_bound}"
        );
    }

    let package: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repository_root().join("browser/package.json"))
            .expect("browser package metadata"),
    )
    .expect("browser package JSON");
    let dependencies = package["dependencies"]
        .as_object()
        .expect("closed production dependency object");
    assert_eq!(dependencies.len(), 1);
    assert_eq!(
        dependencies
            .get("idiomorph")
            .and_then(serde_json::Value::as_str),
        Some("0.7.4")
    );
}

#[test]
fn unattended_commands_do_not_blanket_deny_warnings() {
    for relative in ["scripts/gate.sh", "Cargo.toml", "fuzz/Cargo.toml"] {
        let source = std::fs::read_to_string(repository_root().join(relative)).expect(relative);
        assert!(
            !source.contains("-- -D warnings"),
            "blanket warning denial in {relative}"
        );
        assert!(
            !source.contains("-Dwarnings"),
            "blanket warning denial in {relative}"
        );
    }
}
