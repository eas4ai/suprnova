//! Workspace-level contract tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use suprnova_live::{ENGINE_VERSION, SUPPORTED_PROTOCOL_VERSIONS, SUPPORTED_SNAPSHOT_VERSIONS};

fn live_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--format-version", "1", "--no-deps"])
        .current_dir(live_root())
        .output()
        .expect("cargo metadata must start");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON");
    PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .expect("cargo metadata must expose workspace_root"),
    )
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
    let live_root = live_root();
    let workspace_root = workspace_root();
    let workspace_manifest = read_manifest(&workspace_root.join("Cargo.toml"));
    let live_prefix = live_root
        .strip_prefix(&workspace_root)
        .expect("the Live crate must be inside the Suprnova workspace");

    for package in [
        "crates/suprnova-live-macro-fixture",
        "crates/suprnova-live-test-support",
    ] {
        let workspace_member = live_prefix.join(package);
        let workspace_member = workspace_member
            .to_str()
            .expect("workspace member paths must be UTF-8")
            .replace('\\', "/");
        assert!(
            workspace_manifest.contains(&workspace_member),
            "workspace is missing {workspace_member}"
        );
        assert!(live_root.join(package).join("Cargo.toml").is_file());
    }

    let production_macros = workspace_root.join("suprnova-macros");
    assert!(
        workspace_manifest.contains("\"suprnova-macros\""),
        "workspace is missing the production Suprnova macro package"
    );
    assert!(production_macros.join("Cargo.toml").is_file());
    assert!(production_macros.join("src/live/mod.rs").is_file());
    assert!(
        !live_root.join("crates/suprnova-live-macros").exists(),
        "the retired duplicate Live macro package must not remain"
    );
}

#[test]
fn helper_packages_are_non_publishable_and_featureless() {
    let root = live_root();

    for package in [
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
