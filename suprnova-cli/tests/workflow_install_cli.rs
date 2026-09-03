//! `suprnova workflow:install` must not write through planted symlinks.
//!
//! Print a message via `ui::error` and `std::process::exit(1)` is the
//! contract for every failure path in workflow_install.rs.
//!
//! Teeth: with `fs::write` in place of the contained writer, the dangling
//! link below causes the migration template to be created at the link
//! target, outside the project. The assertions require a non-zero exit,
//! a message naming the file, AND the absence of the victim file - the
//! last is what proves nothing was written through the link.

use std::fs;
use std::process::{Command, Output};

use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// Combined stdout + stderr, since the `ui` helpers may write to either stream.
fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// A symlink standing where a workflow migration file goes must not
/// redirect the write to its target.
#[cfg(unix)]
#[test]
fn workflow_install_does_not_write_through_a_planted_symlink() {
    let dir = tempdir().expect("create tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src/migrations")).expect("create project layout");

    // Dangling on purpose: an existing target would trip the
    // `already exists` guard before any write is attempted, while a
    // dangling link reaches the writer - which must refuse it.
    let victim = root.join("precious.txt");
    std::os::unix::fs::symlink(
        &victim,
        root.join("src/migrations/m20240101_000003_create_workflows_table.rs"),
    )
    .expect("plant symlink");

    let out = Command::new(BIN)
        .arg("workflow:install")
        .current_dir(root)
        .output()
        .expect("spawn suprnova binary");

    let text = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(1),
        "workflow:install must refuse the symlinked path with exit 1; output: {text}"
    );
    assert!(
        text.contains("m20240101_000003_create_workflows_table"),
        "the error must name the file; got: {text}"
    );
    assert!(
        !victim.exists(),
        "workflow:install wrote through the symlink; output: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}
