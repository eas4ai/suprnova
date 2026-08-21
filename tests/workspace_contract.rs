//! Workspace-level contract tests.

use suprnova_live::{ENGINE_VERSION, SUPPORTED_PROTOCOL_VERSIONS, SUPPORTED_SNAPSHOT_VERSIONS};

#[test]
fn exposes_the_iteration_001_version_contract() {
    assert_eq!(ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(SUPPORTED_SNAPSHOT_VERSIONS, &[1]);
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1]);
}
