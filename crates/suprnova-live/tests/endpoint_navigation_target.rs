//! Browser-parity security contract for engine-owned navigation targets.

use suprnova_live::endpoint::EndpointNavigationTarget;

#[test]
fn safe_navigation_targets_are_root_relative_and_bounded() {
    for target in [
        "/catalog/books",
        "/catalog/rust%20books?page=2&q=red+shoes",
        "/catalog/books?literal_dot=%2E%2E",
        "/catalog/books#details",
    ] {
        assert_eq!(
            EndpointNavigationTarget::parse(target)
                .expect("safe browser target")
                .as_str(),
            target
        );
    }
}

#[test]
fn unsafe_navigation_targets_match_browser_rejection_contract() {
    let oversized = format!("/{}", "a".repeat(2_048));
    for target in [
        "catalog/books",
        "//evil.test/catalog",
        "/catalog\\admin",
        "/catalog/../admin",
        "/catalog/./admin",
        "/catalog/%2e%2e/admin",
        "/catalog/.%2E/admin",
        "/catalog/%2fadmin",
        "/catalog/%5cadmin",
        "/catalog/%",
        "/catalog/%2",
        "/catalog/%zz",
        "/catalog/\u{0000}admin",
        oversized.as_str(),
    ] {
        assert!(
            EndpointNavigationTarget::parse(target).is_err(),
            "unsafe browser target must be rejected: {target:?}"
        );
    }
}
