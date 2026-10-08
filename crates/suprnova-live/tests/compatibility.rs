//! Explicit Live protocol/runtime/snapshot compatibility-window tests.

use suprnova_live::protocol::{CompatibilityDecision, CompatibilityWindow, VersionSet};

#[test]
fn v1_triplet_is_supported_and_breaking_versions_request_one_refresh() {
    let window = CompatibilityWindow::v1();
    assert_eq!(
        window.evaluate(VersionSet::new(1, 1, 1)),
        CompatibilityDecision::Compatible
    );
    for versions in [
        VersionSet::new(2, 1, 1),
        VersionSet::new(1, 2, 1),
        VersionSet::new(1, 1, 2),
    ] {
        assert_eq!(
            window.evaluate(versions),
            CompatibilityDecision::RefreshDocument
        );
    }
}
