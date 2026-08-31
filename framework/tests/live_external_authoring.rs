//! Downstream authoring must require only the public `suprnova` crate.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/live-authoring")
}

fn cargo_check(target: &str, target_dir: &Path) -> Output {
    Command::new(env!("CARGO"))
        .args(["check", "--quiet", target])
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_BUILD_JOBS", "1")
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(fixture())
        .output()
        .expect("run downstream cargo check")
}

fn fresh_target() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("suprnova-live-authoring-")
        .tempdir()
        .expect("create isolated downstream target")
}

#[test]
fn external_view_authoring_is_downstream_only_and_fail_closed() {
    let fixture = fixture();
    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).expect("fixture manifest");
    let source = fs::read_to_string(fixture.join("src/lib.rs")).expect("fixture source");

    let dependencies = manifest
        .split_once("[dependencies]")
        .expect("dependencies table")
        .1
        .split("[[")
        .next()
        .expect("dependency entries");
    assert_eq!(
        dependencies
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1
    );
    assert!(manifest.contains("suprnova = { path = \"../../..\" }"));
    for forbidden in ["suprnova-live", "askama", "askama_parser"] {
        assert!(!manifest.contains(forbidden), "manifest named {forbidden}");
    }
    for forbidden in ["suprnova_live", "askama::", "askama_parser"] {
        assert!(!source.contains(forbidden), "source named {forbidden}");
    }

    let target = fresh_target();
    let output = cargo_check("--lib", target.path());
    assert!(
        output.status.success(),
        "external authoring failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (bin, expected) in [
        ("internal-engine", "suprnova_live"),
        ("internal-parser", "askama_parser"),
        ("trusted-html-bypass", "TrustedHtml"),
        ("invalid-view-attribute", "path"),
        ("windows-path-traversal", "view path"),
    ] {
        let output = cargo_check(&format!("--bin={bin}"), target.path());
        assert!(!output.status.success(), "{bin} unexpectedly compiled");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "{bin} did not emit the expected diagnostic `{expected}`:\n{stderr}"
        );
    }
}
