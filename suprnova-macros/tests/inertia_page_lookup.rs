//! `inertia_response!` reads its page lookup from the application crate's
//! own `Cargo.toml`, so only a real `cargo check` of a crate carrying the
//! table proves it: trybuild generates its own manifest and cannot carry
//! `[package.metadata]`.
//!
//! Each test copies the fixture workspace under this package's target
//! directory and checks one of its members. The copies share one target
//! directory, so the framework is built once and the later checks only
//! compile the small fixture crates.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn macros_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scratch_root() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("inertia-page-lookup")
}

/// A fresh copy of the fixture workspace for one test, never the system
/// temp dir: the shared target holds a whole build of `suprnova`.
fn workspace(name: &str) -> PathBuf {
    let dir = scratch_root().join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the previous fixture copy");
    }
    copy_tree(
        &macros_dir().join("tests/fixtures/inertia-page-lookup"),
        &dir,
    );

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
             members = [\"angular\", \"invalid\", \"starter\"]\n\
             \n\
             [workspace.dependencies]\n\
             suprnova = {{ path = {framework:?}, default-features = false }}\n"
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

fn cargo_check(workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO"))
        .args(["check", "--quiet"])
        .args(args)
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

const ANGULAR: &[&str] = &["-p", "inertia-lookup-angular"];

#[test]
fn configured_lookup_accepts_exactly_the_pages_at_the_resolved_paths() {
    let workspace = workspace("configured");

    let lib = cargo_check(&workspace, &[ANGULAR, &["--lib"]].concat());
    assert_compiles(
        &lib,
        "resources/angular/pages/Tramits/index.page.ts and \
         resources/angular/pages/Tramits/BaixaMatricula/create.page.ts exist",
    );

    let missing = cargo_check(&workspace, &[ANGULAR, &["--bin", "missing-page"]].concat());
    assert_rejected(
        &missing,
        &[
            "Inertia component 'Tramits/BaixaMatricula/Edit' not found.",
            "Looked for: resources/angular/pages/Tramits/BaixaMatricula/edit.page.ts",
            "[package.metadata.suprnova.inertia] in Cargo.toml",
        ],
    );

    let starter = cargo_check(
        &workspace,
        &[ANGULAR, &["--bin", "starter-location"]].concat(),
    );
    assert_rejected(
        &starter,
        &[
            "Inertia component 'Home' not found.",
            "Looked for: resources/angular/pages/home.page.ts",
        ],
    );
}

#[test]
fn editing_a_page_or_the_table_re_runs_the_check() {
    let workspace = workspace("tracking");
    let lib = [ANGULAR, &["--lib"]].concat();
    assert_compiles(&cargo_check(&workspace, &lib), "every page exists");

    // Nothing in the crate's sources changes: only the page goes away.
    let page = workspace.join("angular/resources/angular/pages/Tramits/index.page.ts");
    let body = fs::read(&page).expect("read the page");
    fs::remove_file(&page).expect("delete the page");
    assert_rejected(
        &cargo_check(&workspace, &lib),
        &["Looked for: resources/angular/pages/Tramits/index.page.ts"],
    );

    fs::write(&page, body).expect("restore the page");
    assert_compiles(&cargo_check(&workspace, &lib), "the page is back");

    // Only the table changes: the pages now resolve to files that do not exist.
    let manifest = workspace.join("angular/Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("read the fixture manifest");
    fs::write(
        &manifest,
        text.replace("{name|lower}.page.ts", "{name|kebab}.component.ts"),
    )
    .expect("edit the lookup table");
    assert_rejected(
        &cargo_check(&workspace, &lib),
        &["Looked for: resources/angular/pages/Tramits/index.component.ts"],
    );
}

#[test]
fn a_malformed_table_is_a_compile_error_naming_the_key() {
    let workspace = workspace("malformed");
    let output = cargo_check(&workspace, &["-p", "inertia-lookup-invalid", "--lib"]);
    assert_rejected(
        &output,
        &[
            "[package.metadata.suprnova.inertia] in Cargo.toml",
            "`page_file`",
            "unknown filter `upper`",
        ],
    );
}

#[test]
fn without_the_table_the_starter_lookup_is_unchanged() {
    let workspace = workspace("starter");

    let lib = cargo_check(&workspace, &["-p", "inertia-lookup-starter", "--lib"]);
    assert_compiles(
        &lib,
        "frontend/src/pages/Home.svelte and frontend/src/pages/Users/Index.tsx exist",
    );

    let missing = cargo_check(
        &workspace,
        &["-p", "inertia-lookup-starter", "--bin", "missing-page"],
    );
    assert_rejected(
        &missing,
        &[
            "Inertia component 'Missing' not found.",
            "Looked in: frontend/src/pages/",
            "Tried extensions: .svelte, .tsx, .jsx, .vue",
            "Available components:",
            "  - Home",
            "  - Users/Index",
        ],
    );
}
