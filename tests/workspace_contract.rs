//! Workspace-level contract tests.

use std::fs;
use std::path::{Path, PathBuf};

use suprnova_live::{ENGINE_VERSION, SUPPORTED_PROTOCOL_VERSIONS, SUPPORTED_SNAPSHOT_VERSIONS};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn exposes_the_kernel_version_contract() {
    assert_eq!(ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
    assert_eq!(SUPPORTED_SNAPSHOT_VERSIONS, &[1]);
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);
}

#[test]
fn workspace_declares_the_internal_kernel_packages() {
    let root = repository_root();
    let workspace_manifest = read_manifest(&root.join("Cargo.toml"));

    for package in [
        "crates/suprnova-live-macros",
        "crates/suprnova-live-macro-fixture",
        "crates/suprnova-live-test-support",
    ] {
        assert!(
            workspace_manifest.contains(package),
            "workspace is missing {package}"
        );
        assert!(root.join(package).join("Cargo.toml").is_file());
    }
}

#[test]
fn helper_packages_are_non_publishable_and_featureless() {
    let root = repository_root();

    for package in [
        "crates/suprnova-live-macros",
        "crates/suprnova-live-macro-fixture",
        "crates/suprnova-live-test-support",
    ] {
        let manifest = read_manifest(&root.join(package).join("Cargo.toml"));
        assert!(
            manifest.contains("publish = false"),
            "{package} is publishable"
        );
        assert!(
            manifest.contains("[features]\ndefault = []"),
            "{package} has default feature drift"
        );
    }
}
