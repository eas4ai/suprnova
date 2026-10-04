//! Fixtures that compile with the dependencies of an application crate and
//! then run: macro output that must build without extra dependencies, and
//! macro output whose failure path must return an error instead of panicking.

use std::path::{Path, PathBuf};

#[test]
fn macro_output_compiles_and_runs_in_an_application_crate() {
    install_trybuild_templates();
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/runtime/pass/*.rs");
}

/// `inertia_response!` checks its page exists under the crate it expands
/// in, which for a fixture is trybuild's generated crate.
fn install_trybuild_templates() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR sits inside the target directory")
        .to_path_buf();
    let destination = target.join("tests/trybuild/suprnova-macros/frontend");
    copy_tree(
        &manifest.join("tests/fixtures/runtime-frontend"),
        &destination,
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create trybuild page directory");
    for entry in std::fs::read_dir(source).expect("read page fixtures") {
        let entry = entry.expect("read page fixture");
        let destination = destination.join(entry.file_name());
        if entry.file_type().expect("inspect page fixture").is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).expect("copy page fixture");
        }
    }
}
