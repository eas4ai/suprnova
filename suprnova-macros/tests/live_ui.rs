//! Compile-time contracts for Live component authoring through `suprnova::live`.

use std::path::{Path, PathBuf};

#[test]
fn live_component_authoring_contract() {
    install_trybuild_templates();
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/live/pass/*.rs");
    tests.compile_fail("tests/ui/live/fail/*.rs");
}

fn install_trybuild_templates() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().expect("workspace root");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target"));
    let destination = target.join("tests/trybuild/suprnova-macros/templates");
    copy_tree(&manifest.join("tests/templates"), &destination);
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create trybuild template directory");
    for entry in std::fs::read_dir(source).expect("read macro template fixtures") {
        let entry = entry.expect("read macro template fixture");
        let destination = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("inspect macro template fixture")
            .is_dir()
        {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).expect("copy trybuild template fixture");
        }
    }
}
