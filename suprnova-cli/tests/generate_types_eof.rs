//! `suprnova generate-types` writes a file that ends in exactly one newline.
//!
//! The v2.0.0 binary ended `inertia-props.ts` in two, because the blank
//! line that separates one interface from the next was also written after
//! the last one. A project that enforces `git diff --check` then failed
//! with `new blank line at EOF` on a file it cannot correct by hand: the
//! next regeneration writes the blank line straight back. This drives the
//! real binary through the reported reproduction (GitHub issue #5).

use std::fs;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// The issue's disposable project: a minimal manifest and one prop struct.
fn seed_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("create workspace tempdir");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"audit\"\n",
    )
    .expect("write manifest");
    fs::create_dir_all(dir.path().join("src")).expect("create src");
    fs::write(
        dir.path().join("src/lib.rs"),
        "#[derive(InertiaProps)]\npub struct AuditProps {\n    pub title: String,\n}\n",
    )
    .expect("write props");
    dir
}

fn generate_types_in(dir: &tempfile::TempDir) -> Vec<u8> {
    let out = Command::new(BIN)
        .arg("generate-types")
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova binary");
    assert!(
        out.status.success(),
        "generate-types must succeed; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read(dir.path().join("frontend/src/types/inertia-props.ts")).expect("read generated file")
}

#[test]
fn the_generated_file_ends_with_exactly_one_newline() {
    let dir = seed_project();
    let bytes = generate_types_in(&dir);
    let text = String::from_utf8(bytes.clone()).expect("generated TypeScript is UTF-8");

    assert!(text.contains("export interface AuditProps"), "{text}");
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "the file must end with a newline:\n{text:?}"
    );
    assert!(
        !bytes.ends_with(b"\n\n"),
        "the file must not end with a blank line (`git diff --check` reports \
         `new blank line at EOF` for it):\n{text:?}"
    );
}

#[test]
fn removing_the_last_struct_leaves_a_header_that_ends_with_one_newline() {
    // The empty file is the other shape the writer produces, and its header
    // used to end in the same blank line a struct did.
    let dir = seed_project();
    generate_types_in(&dir);
    fs::write(dir.path().join("src/lib.rs"), "pub struct Plain;\n").expect("remove the derive");

    let bytes = generate_types_in(&dir);
    let text = String::from_utf8(bytes.clone()).expect("generated TypeScript is UTF-8");
    assert!(
        !text.contains("AuditProps"),
        "the stale declaration is gone:\n{text}"
    );
    assert_eq!(bytes.last(), Some(&b'\n'), "{text:?}");
    assert!(!bytes.ends_with(b"\n\n"), "{text:?}");
}
