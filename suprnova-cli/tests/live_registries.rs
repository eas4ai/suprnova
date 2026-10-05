//! `registries`: the project file, the library tree, fetching at a tag, the
//! hash, the signature, pins and rotation, the plan and the record
//! (REG-001 to REG-004, REG-006 to REG-015, REG-019, REG-020, REG-023 to
//! REG-029, REG-033). Each test is named for the falsifier it closes.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// A bare project root: `live:add` needs only `Cargo.toml` to see one.
fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    tmp
}

fn live_add(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("live:add")
        .args(args)
        .current_dir(root)
        .env_remove("SUPRNOVA_LIBRARY_KEY")
        .output()
        .expect("run live:add")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// REG-001: a project holding `Suprnova.toml` and no `suprnova.toml` makes
/// `live:add` fail with a message that names the rename.
#[test]
fn reg_001_a_project_holding_only_the_legacy_project_file_is_refused_naming_the_rename() {
    let root = project();
    fs::write(root.path().join("Suprnova.toml"), "[serve]\n").expect("write");
    let output = live_add(root.path(), &["button"]);
    let text = combined(&output);
    assert!(
        !output.status.success(),
        "live:add installed over a legacy project file:\n{text}"
    );
    assert!(
        text.contains("rename `Suprnova.toml` to `suprnova.toml`"),
        "the refusal does not name the rename:\n{text}"
    );
    assert!(
        !root.path().join("templates").exists(),
        "a refused install wrote files"
    );
}
