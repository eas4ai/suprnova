//! Stable error taxonomy and redaction contract tests.

use std::io;

use suprnova_live::error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};

#[test]
fn production_formatting_exposes_only_closed_safe_fields() {
    let error = LiveError::new(
        ErrorCategory::Snapshot,
        RecoveryInstruction::RefreshIsland,
        SafeDiagnosticCode::SignatureInvalid,
    )
    .with_source(io::Error::other(
        "secret=snapshot-body signature=do-not-print",
    ));

    let display = error.to_string();
    let debug = format!("{error:?}");

    assert_eq!(
        display,
        "snapshot:signature_invalid recovery=refresh_island"
    );
    assert_eq!(debug, display);
    assert!(!display.contains("secret"));
    assert!(!debug.contains("do-not-print"));
    assert_eq!(error.category(), ErrorCategory::Snapshot);
    assert_eq!(error.recovery(), RecoveryInstruction::RefreshIsland);
    assert_eq!(error.detail(), SafeDiagnosticCode::SignatureInvalid);
}

#[test]
fn stable_error_fields_use_snake_case_machine_values() {
    assert_eq!(ErrorCategory::SizeLimit.as_str(), "size_limit");
    assert_eq!(RecoveryInstruction::RetainDom.as_str(), "retain_dom");
    assert_eq!(SafeDiagnosticCode::DuplicateKey.as_str(), "duplicate_key");
}
