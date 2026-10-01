//! Proves that the Live packages are integrated into the public Suprnova workspace.

use std::{fs, path::PathBuf, process::Command};

use serde_json::Value;

#[test]
fn live_packages_share_the_suprnova_workspace_without_a_framework_cycle() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata must start from the Live manifest directory");
    assert!(
        output.status.success(),
        "cargo metadata failed from {}: {}",
        env!("CARGO_MANIFEST_DIR"),
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON");
    let reported_workspace_root = PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .expect("cargo metadata workspace_root must be a string"),
    );
    assert!(
        reported_workspace_root.is_absolute(),
        "cargo metadata workspace_root must be absolute, got {}",
        reported_workspace_root.display()
    );
    let workspace_root = fs::canonicalize(&reported_workspace_root).unwrap_or_else(|error| {
        panic!(
            "cargo metadata workspace_root {} must be canonicalizable: {error}",
            reported_workspace_root.display()
        )
    });
    assert_eq!(
        reported_workspace_root, workspace_root,
        "cargo metadata workspace_root must be canonical"
    );
    let live_root = workspace_root.join("crates/suprnova-live");
    assert!(
        !live_root.join("rust-toolchain.toml").exists(),
        "{} must not own a nested Rust toolchain; the Suprnova workspace root is authoritative",
        live_root.display()
    );
    assert!(
        !live_root.join("Cargo.lock").exists(),
        "{} must not retain a nested lockfile; the Suprnova workspace lock is authoritative",
        live_root.display()
    );

    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata packages must be an array");
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .expect("cargo metadata workspace_members must be an array");
    let framework = packages
        .iter()
        .find(|package| package["name"] == "suprnova")
        .expect("the public suprnova package must be present in cargo metadata");

    for (name, expected_manifest_root) in [
        ("suprnova-live", live_root.clone()),
        ("suprnova-macros", workspace_root.join("suprnova-macros")),
        ("suprnova-live-macro-fixture", live_root.clone()),
        ("suprnova-live-test-support", live_root.clone()),
    ] {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap_or_else(|| {
                panic!("integrated package {name} must be present in cargo metadata")
            });
        let package_id = package["id"]
            .as_str()
            .unwrap_or_else(|| panic!("integrated package {name} must have a string package ID"));
        assert!(
            workspace_members
                .iter()
                .any(|member| member.as_str() == Some(package_id)),
            "integrated package {name} ({package_id}) must be a root workspace member"
        );

        for field in ["version", "edition", "rust_version", "license"] {
            let expected = framework[field].as_str().unwrap_or_else(|| {
                panic!("public suprnova package must have a string {field} value")
            });
            let actual = package[field].as_str().unwrap_or_else(|| {
                panic!("integrated package {name} must have a string {field} value")
            });
            assert_eq!(
                actual, expected,
                "integrated package {name} must inherit public suprnova {field}"
            );
        }

        let reported_manifest = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .unwrap_or_else(|| panic!("integrated package {name} must have a manifest path")),
        );
        let manifest = fs::canonicalize(&reported_manifest).unwrap_or_else(|error| {
            panic!(
                "integrated package {name} manifest {} must be canonicalizable: {error}",
                reported_manifest.display()
            )
        });
        assert!(
            manifest.starts_with(&expected_manifest_root),
            "integrated package {name} manifest {} must be beneath {}",
            manifest.display(),
            expected_manifest_root.display()
        );
    }

    let engine = packages
        .iter()
        .find(|package| package["name"] == "suprnova-live")
        .expect("the integrated suprnova-live engine must be present");
    let has_framework_dependency = engine["dependencies"]
        .as_array()
        .expect("the suprnova-live dependency list must be an array")
        .iter()
        .any(|dependency| dependency["name"] == "suprnova");
    assert!(
        !has_framework_dependency,
        "suprnova-live must remain host-neutral and must not depend on the public suprnova package"
    );
}
