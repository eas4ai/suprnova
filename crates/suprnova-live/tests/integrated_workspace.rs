//! Proves that the Live packages are integrated into the public Suprnova workspace.

use std::{env, path::PathBuf, process::Command};

use serde_json::Value;

#[test]
fn live_packages_share_the_suprnova_workspace_without_a_framework_cycle() {
    let workspace_root = PathBuf::from(
        env::var("SUPRNOVA_WORKSPACE_ROOT")
            .expect("SUPRNOVA_WORKSPACE_ROOT must identify the integration worktree"),
    );
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(&workspace_root)
        .output()
        .expect("cargo metadata must start");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata packages must be an array");
    let framework_version = packages
        .iter()
        .find(|package| package["name"] == "suprnova")
        .and_then(|package| package["version"].as_str())
        .expect("the public suprnova package must be present");
    let live_root = workspace_root.join("crates/suprnova-live");

    for name in [
        "suprnova-live",
        "suprnova-live-macros",
        "suprnova-live-macro-fixture",
        "suprnova-live-test-support",
    ] {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| panic!("missing integrated package {name}"));
        assert_eq!(package["version"], framework_version);
        let manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .expect("manifest_path must be a string"),
        );
        assert!(manifest.starts_with(&live_root));
    }

    let engine = packages
        .iter()
        .find(|package| package["name"] == "suprnova-live")
        .expect("the integrated engine must be present");
    let has_framework_dependency = engine["dependencies"]
        .as_array()
        .expect("dependencies must be an array")
        .iter()
        .any(|dependency| dependency["name"] == "suprnova");
    assert!(!has_framework_dependency);
}
