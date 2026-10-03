//! `#[model]` and `#[suprnova::main]` read app-wide defaults from the
//! application crate's own `Cargo.toml`, so only a real build of a crate
//! carrying the tables proves them: trybuild generates its own manifest and
//! cannot carry `[package.metadata]`.
//!
//! Each test copies the fixture workspace under this package's target
//! directory and builds one of its members. The copies share one target
//! directory, so the framework is built once.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn macros_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_root() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("app-defaults")
}

/// A fresh copy of the fixture workspace for one test.
fn workspace(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the previous fixture copy");
    }
    copy_tree(&macros_dir().join("tests/fixtures/app-defaults"), &dir);

    let framework = macros_dir()
        .join("../framework")
        .canonicalize()
        .expect("locate the framework crate");
    let framework = framework.to_str().expect("a UTF-8 framework path");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\n\
             resolver = \"3\"\n\
             members = [\"invalid\", \"laravel\", \"plain\"]\n\
             \n\
             [workspace.dependencies]\n\
             suprnova = {{ path = {framework:?}, default-features = false }}\n\
             serde = {{ version = \"1\", features = [\"derive\"] }}\n\
             chrono = \"0.4\"\n\
             sea-orm = {{ version = \"2.0\", features = [\"macros\", \"with-chrono\"] }}\n\
             tokio = {{ version = \"1\", features = [\"full\"] }}\n"
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

fn cargo(workspace: &Path, command: &str, package: &str) -> Output {
    Command::new(env!("CARGO"))
        .args([command, "--quiet", "-p", package])
        .env("CARGO_TARGET_DIR", scratch_root().join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .current_dir(workspace)
        .output()
        .expect("run cargo on the fixture workspace")
}

fn assert_success(output: &Output, why: &str) {
    assert!(
        output.status.success(),
        "{why}, yet it failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(output: &Output, expected: &[&str]) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the build passed, expected it to fail naming {expected:?}:\n{stderr}"
    );
    for fragment in expected {
        assert!(
            stderr.contains(fragment),
            "expected `{fragment}` in the compile error:\n{stderr}"
        );
    }
}

#[test]
fn the_tables_set_native_date_times_and_unsigned_ids() {
    let workspace = workspace("laravel");
    assert_success(
        &cargo(&workspace, "run", "app-defaults-laravel"),
        "the manifest sets datetime_cast = \"native\" and unsigned_ids = true",
    );
}

#[test]
fn without_the_tables_the_defaults_are_unchanged() {
    let workspace = workspace("plain");
    assert_success(
        &cargo(&workspace, "run", "app-defaults-plain"),
        "without the tables date-times are text and ids are signed",
    );
}

#[test]
fn editing_the_table_rebuilds_the_models() {
    let workspace = workspace("tracking");
    assert_success(
        &cargo(&workspace, "check", "app-defaults-laravel"),
        "the native casts apply",
    );

    // Only the table changes: the date-times go back to the text cast, and
    // the fixture's `storage` no longer type-checks.
    let manifest = workspace.join("laravel/Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("read the fixture manifest");
    fs::write(&manifest, text.replace("\"native\"", "\"text\"")).expect("edit the table");
    assert_rejected(
        &cargo(&workspace, "check", "app-defaults-laravel"),
        &["mismatched types"],
    );
}

#[test]
fn a_malformed_table_is_a_compile_error_naming_the_key() {
    let workspace = workspace("malformed");
    assert_rejected(
        &cargo(&workspace, "check", "app-defaults-invalid"),
        &[
            "[package.metadata.suprnova.model] in Cargo.toml: \
             `datetime_cast` must be \"text\" or \"native\", got \"Native\"",
            "[package.metadata.suprnova.schema] in Cargo.toml: \
             `unsigned_ids` must be true or false",
        ],
    );
}
