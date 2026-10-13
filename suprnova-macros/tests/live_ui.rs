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
    // trybuild builds its fixture crate under the target directory cargo
    // reports, which honours `build.target-dir` in a config file as well as
    // the environment. `CARGO_TARGET_TMPDIR` is that directory's `tmp/`
    // child, set by cargo when it compiles an integration test, so its parent
    // is the same directory trybuild resolves; an env-var lookup with an
    // in-tree `target/` fallback put the templates where trybuild never looked
    // once the target moved through config (2026-09-21).
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR sits inside the target directory")
        .to_path_buf();
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
