//! `suprnova generate-types` must not claim it generated a file it left
//! alone.
//!
//! The generator writes only when the emitted content differs from what is
//! on disk, which is what stops `serve`'s backend watcher from restarting on
//! a no-op regeneration. Reporting every run as "Generated" hid that: the
//! user was told a file changed when the tool had deliberately not touched
//! it. These drive the real binary, because the claim is the printed line.

use std::fs;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// A minimal project: a manifest so `generate-types` accepts the directory,
/// one prop struct, and one Fluent catalog so both artifacts are exercised.
fn seed_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("create workspace tempdir");
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"smoke\"\n",
    )
    .expect("write manifest");
    fs::create_dir_all(dir.path().join("src")).expect("create src");
    fs::write(
        dir.path().join("src/props.rs"),
        "#[derive(InertiaProps)]\npub struct HomeProps {\n    pub title: String,\n}\n",
    )
    .expect("write props");
    fs::create_dir_all(dir.path().join("lang/en")).expect("create lang/en");
    fs::write(dir.path().join("lang/en/app.ftl"), "welcome = Hello\n").expect("write catalog");
    dir
}

fn generate_types_in(dir: &tempfile::TempDir) -> String {
    let out = Command::new(BIN)
        .arg("generate-types")
        .current_dir(dir.path())
        .output()
        .expect("spawn suprnova binary");
    assert!(
        out.status.success(),
        "generate-types must succeed; output: {}",
        combined(&out)
    );
    combined(&out)
}

#[test]
fn the_first_run_reports_both_artifacts_as_generated() {
    let dir = seed_project();
    let text = generate_types_in(&dir);

    assert!(
        text.contains("Generated ./frontend/src/types/inertia-props.ts"),
        "a first run really does write the file; got: {text}"
    );
    assert!(
        text.contains("Generated ./frontend/src/types/lang-keys.ts"),
        "a first run really does write the catalog types; got: {text}"
    );
    assert!(
        !text.contains("up to date"),
        "nothing was up to date on a first run; got: {text}"
    );
}

#[test]
fn an_identical_rerun_reports_both_artifacts_as_up_to_date() {
    let dir = seed_project();
    generate_types_in(&dir);
    let text = generate_types_in(&dir);

    assert!(
        text.contains("./frontend/src/types/inertia-props.ts is up to date"),
        "a rerun that wrote nothing must say so; got: {text}"
    );
    assert!(
        text.contains("./frontend/src/types/lang-keys.ts is up to date"),
        "the catalog types are up to date too; got: {text}"
    );
    assert!(
        !text.contains("Generated ./frontend/src/types/"),
        "nothing was generated, so nothing may claim it was; got: {text}"
    );
}

#[test]
fn a_real_edit_is_reported_as_generated_again() {
    let dir = seed_project();
    generate_types_in(&dir);
    fs::write(
        dir.path().join("src/props.rs"),
        "#[derive(InertiaProps)]\npub struct HomeProps {\n    pub title: String,\n    pub n: i64,\n}\n",
    )
    .expect("edit props");

    let text = generate_types_in(&dir);
    assert!(
        text.contains("Generated ./frontend/src/types/inertia-props.ts"),
        "a changed shape must be written and reported; got: {text}"
    );
    assert!(
        text.contains("./frontend/src/types/lang-keys.ts is up to date"),
        "the untouched catalog must not be reported as regenerated; got: {text}"
    );
}

#[test]
fn removing_the_final_props_type_reports_one_mutation_then_up_to_date() {
    let dir = seed_project();
    generate_types_in(&dir);
    fs::write(
        dir.path().join("src/props.rs"),
        "pub struct PlainRustType;\n",
    )
    .expect("remove final props derive");

    let changed = generate_types_in(&dir);
    assert!(
        changed.contains("No InertiaProps structs found."),
        "the empty scan remains visible; got: {changed}"
    );
    assert!(
        changed.contains("Generated ./frontend/src/types/inertia-props.ts"),
        "removing stale declarations changed the artifact; got: {changed}"
    );
    assert!(
        !fs::read_to_string(dir.path().join("frontend/src/types/inertia-props.ts"))
            .expect("read emptied output")
            .contains("HomeProps"),
        "the stale declaration must be removed"
    );

    let unchanged = generate_types_in(&dir);
    assert!(
        unchanged.contains("./frontend/src/types/inertia-props.ts is up to date"),
        "the identical empty artifact is unchanged; got: {unchanged}"
    );
    assert!(
        !unchanged.contains("Generated ./frontend/src/types/inertia-props.ts"),
        "an empty no-op must not claim a mutation; got: {unchanged}"
    );
}
