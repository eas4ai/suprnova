//! A fresh scaffold is rustfmt-clean (PAR-080): a developer's first
//! `cargo fmt` changes nothing they did not write.
//!
//! The shared backend templates, the Live kit and the API starter are
//! scaffolded with `suprnova new` and checked with `cargo fmt --check`, which
//! reads the project's own `edition = "2024"` and so applies the 2024 style
//! edition the generated project formats under. `cargo fmt` resolves the
//! manifest without dependencies, so the check needs no network and no
//! `Cargo.lock`.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// Run `suprnova new <name> <args>` inside `tmp` and return the project root.
fn scaffold(tmp: &TempDir, name: &str, args: &[&str]) -> PathBuf {
    let output = Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .arg("new")
        .arg(name)
        .arg("--no-interaction")
        .arg("--no-git")
        .args(args)
        .current_dir(tmp.path())
        .output()
        .expect("`suprnova new` runs");
    assert!(
        output.status.success(),
        "`suprnova new {name} {args:?}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    tmp.path().join(name)
}

/// `cargo fmt --check` in `project`: `Ok(())` when it finished and found
/// nothing to change; otherwise its exit status with everything it printed,
/// so a formatter that found differences, failed before printing them or
/// was killed never reads as clean.
fn fmt_check(project: &Path) -> Result<(), String> {
    let output = Command::new(env!("CARGO"))
        .args(["fmt", "--check"])
        .current_dir(project)
        .output()
        .expect("`cargo fmt --check` runs");
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "`cargo fmt --check` exited with {}\nstdout:\n{}stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn assert_fmt_clean(name: &str, args: &[&str]) {
    let tmp = TempDir::new().expect("a temporary directory");
    let project = scaffold(&tmp, name, args);
    if let Err(report) = fmt_check(&project) {
        panic!(
            "a fresh `suprnova new {name} {args:?}` is not rustfmt-clean, or the check did not \
             finish: {report}"
        );
    }
}

#[test]
fn a_svelte_scaffold_is_rustfmt_clean() {
    assert_fmt_clean("fmt_svelte", &["--frontend", "svelte"]);
}

#[test]
fn a_react_scaffold_is_rustfmt_clean() {
    assert_fmt_clean("fmt_react", &["--frontend", "react"]);
}

#[test]
fn a_vue_scaffold_is_rustfmt_clean() {
    assert_fmt_clean("fmt_vue", &["--frontend", "vue"]);
}

#[test]
fn the_live_kit_scaffold_is_rustfmt_clean() {
    assert_fmt_clean("fmt_live", &["--frontend", "live"]);
}

#[test]
fn an_api_scaffold_is_rustfmt_clean() {
    assert_fmt_clean("fmt_api", &["--api"]);
}
