//! Compile-time rules of `#[suprnova::model]` and `#[suprnova::main]` from
//! PAR-044 and PAR-045: the key type comes from the primary-key field, a
//! `key_type` that disagrees with it fails the build naming both, and a
//! `[package.metadata.suprnova]` key or value the framework does not know
//! fails the build of the macro that reads it, naming it.
//!
//! Only a real `cargo check` of a crate proves a build failure, and only a
//! real package can carry `[package.metadata]`, which trybuild's generated
//! manifest cannot. Each test copies the fixture workspace under this
//! package's target directory and checks one member; the copies share one
//! target directory, so the framework is checked once.
//!
//! The tests sit in one module whose name holds `key_type`, so the
//! `par-laravel-defaults` mechanism selects them all by that name.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn macros_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_root() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("model-settings")
}

const MEMBERS: &[&str] = &[
    "inferred",
    "mismatch",
    "unknown-model-key",
    "unknown-model-value",
    "unknown-schema-key",
    "unknown-schema-value",
];

/// A fresh copy of the fixture workspace for one test, never the system
/// temp dir: the shared target holds a check of `suprnova`.
fn workspace(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the previous fixture copy");
    }
    copy_tree(&macros_dir().join("tests/fixtures/model-settings"), &dir);

    let framework = macros_dir()
        .join("../framework")
        .canonicalize()
        .expect("locate the framework crate");
    let framework = framework.to_str().expect("a UTF-8 framework path");
    let members = MEMBERS
        .iter()
        .map(|member| format!("{member:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\n\
             resolver = \"3\"\n\
             members = [{members}]\n\
             \n\
             [workspace.dependencies]\n\
             suprnova = {{ path = {framework:?}, default-features = false }}\n\
             sea-orm = {{ version = \"2.0\", default-features = false }}\n\
             serde = {{ version = \"1\", features = [\"derive\"] }}\n\
             tokio = {{ version = \"1\", features = [\"sync\"] }}\n"
        ),
    )
    .expect("write the fixture workspace manifest");

    // Starting from the repository's lockfile keeps the fixture on the
    // dependency versions the framework is tested with.
    fs::copy(macros_dir().join("../Cargo.lock"), dir.join("Cargo.lock"))
        .expect("seed the fixture lockfile");
    dir
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create the fixture copy");
    for entry in fs::read_dir(source).expect("read the fixture") {
        let entry = entry.expect("read a fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("inspect a fixture entry").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy a fixture file");
        }
    }
}

fn cargo_check(workspace: &Path, member: &str) -> Output {
    Command::new(env!("CARGO"))
        .args([
            "check",
            "--quiet",
            "-p",
            &format!("model-settings-{member}"),
        ])
        .env("CARGO_TARGET_DIR", scratch_root().join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(workspace)
        .output()
        .expect("run cargo check on the fixture workspace")
}

fn assert_compiles(output: &Output, why: &str) {
    assert!(
        output.status.success(),
        "{why}, yet the check failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(output: &Output, expected: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the check passed, expected it to fail naming {expected:?}:\n{stderr}"
    );
    for fragment in expected {
        assert!(
            stderr.contains(fragment),
            "expected `{fragment}` in the compile error:\n{stderr}"
        );
    }
}

mod key_type_and_settings {
    use super::*;

    /// A `u64` key, a `String` key and an `i64` key, none naming
    /// `key_type`, and a `key_type` that spells the field's type another
    /// way: each model's `Key` is its field's type. It fails when a model
    /// without `key_type` still gets `i64`.
    #[test]
    fn key_type_is_inferred_from_the_primary_key_field() {
        let workspace = workspace("inferred");
        assert_compiles(
            &cargo_check(&workspace, "inferred"),
            "each key type is its primary-key field's type",
        );
    }

    /// `key_type = "i64"` on a `u64` key fails the build with a message
    /// naming both types.
    #[test]
    fn a_disagreeing_key_type_fails_naming_both() {
        let workspace = workspace("mismatch");
        assert_rejected(
            &cargo_check(&workspace, "mismatch"),
            &["`key_type = \"i64\"` disagrees with the primary-key field's type `u64`"],
        );
    }

    /// A misspelled key and an unknown value in
    /// `[package.metadata.suprnova.model]` fail the model's build, each
    /// named.
    #[test]
    fn an_unknown_model_setting_fails_the_model_build() {
        let workspace = workspace("unknown-model");
        assert_rejected(
            &cargo_check(&workspace, "unknown-model-key"),
            &[
                "[package.metadata.suprnova.model] in Cargo.toml",
                "unknown key `datetime_casts`",
            ],
        );
        assert_rejected(
            &cargo_check(&workspace, "unknown-model-value"),
            &[
                "[package.metadata.suprnova.model] in Cargo.toml",
                "unknown `datetime_cast` value `zoned`",
            ],
        );
    }

    /// A misspelled key and a value of the wrong type in
    /// `[package.metadata.suprnova.schema]` fail the build of
    /// `#[suprnova::main]`, each named.
    #[test]
    fn an_unknown_schema_setting_fails_the_main_build() {
        let workspace = workspace("unknown-schema");
        assert_rejected(
            &cargo_check(&workspace, "unknown-schema-key"),
            &[
                "[package.metadata.suprnova.schema] in Cargo.toml",
                "unknown key `unsigned_id`",
            ],
        );
        assert_rejected(
            &cargo_check(&workspace, "unknown-schema-value"),
            &[
                "[package.metadata.suprnova.schema] in Cargo.toml",
                "`unsigned_ids` must be `true` or `false`, got `\"yes\"`",
            ],
        );
    }
}
