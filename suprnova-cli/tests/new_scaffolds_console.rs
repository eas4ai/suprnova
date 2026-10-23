//! Smoke test: `suprnova new` must scaffold a working console binary
//! into both flavors of generated project.
//!
//! Pins:
//!   - `src/bin/console.rs` exists and references
//!     `suprnova::console::dispatch_argv`
//!   - `src/commands/mod.rs` exists (empty stub, ready for
//!     `make:command` to append to)
//!   - `Cargo.toml` declares the `console` `[[bin]]` entry
//!   - `src/lib.rs` declares `pub mod commands;`
//!
//! These are file-shape assertions - we don't try to `cargo build` the
//! scaffolded project because it depends on the released `suprnova`
//! crate from crates.io, which would either pull a stale version or
//! fail offline. The existing dogfood path (app/src/bin/console.rs)
//! already proves the wiring compiles and runs end-to-end against
//! HEAD framework code.

use std::process::Command;
use tempfile::TempDir;

fn run_new(cwd: &std::path::Path, name: &str, args: &[&str]) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_suprnova"));
    cmd.arg("new")
        .arg(name)
        .arg("--no-interaction")
        .arg("--no-git");
    for a in args {
        cmd.arg(a);
    }
    let status = cmd
        .current_dir(cwd)
        .status()
        .expect("suprnova binary spawnable");
    assert!(status.success(), "`suprnova new {name}` should succeed");
}

fn read(p: impl AsRef<std::path::Path>) -> String {
    std::fs::read_to_string(p.as_ref())
        .unwrap_or_else(|e| panic!("read {}: {e}", p.as_ref().display()))
}

/// The framework's default features other than `testing`, read from its
/// own manifest, so a default added there cannot be left out of new apps.
fn framework_production_defaults() -> Vec<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../framework/Cargo.toml");
    let manifest: toml::Table = read(&manifest).parse().expect("framework manifest parses");
    manifest["features"]["default"]
        .as_array()
        .expect("the framework declares default features")
        .iter()
        .map(|feature| {
            feature
                .as_str()
                .expect("feature names are strings")
                .to_owned()
        })
        .filter(|feature| feature != "testing")
        .collect()
}

/// Pins the production build shape (design doc section 5.1): the
/// production dependency turns default features off and lists every
/// non-`testing` default, and a dev-dependency turns `testing` back on
/// for tests only.
fn assert_production_build_shape(cargo: &str) {
    let production = cargo
        .split("[dev-dependencies]")
        .next()
        .and_then(|section| section.lines().find(|line| line.starts_with("suprnova = ")))
        .expect("Cargo.toml declares the production suprnova dependency");
    assert!(
        production.contains("default-features = false, features = ["),
        "Cargo.toml's production dependency must turn default features off: {production}"
    );
    let listed: Vec<String> = production
        .split("features = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the production dependency lists its features")
        .split(',')
        .map(|feature| feature.trim().trim_matches('"').to_owned())
        .filter(|feature| !feature.is_empty())
        .collect();
    let mut expected = framework_production_defaults();
    let mut listed_sorted = listed.clone();
    expected.sort();
    listed_sorted.sort();
    assert_eq!(
        listed_sorted, expected,
        "Cargo.toml's production dependency must list every non-testing default \
         of the framework: {production}"
    );
    assert!(
        cargo.contains("[dev-dependencies]"),
        "Cargo.toml declares a [dev-dependencies] table: {cargo}"
    );
    let dev_section = cargo
        .split("[dev-dependencies]")
        .nth(1)
        .expect("[dev-dependencies] table present");
    let dev_dependency = format!(
        "suprnova = {{ git = \"https://github.com/eas4ai/suprnova.git\", {}, ",
        suprnova_cli::templates::framework_pin()
    );
    assert!(
        dev_section.contains(&dev_dependency)
            && dev_section.contains(r#"features = ["testing"] }"#),
        "Cargo.toml's dev-dependency must turn `testing` back on: {cargo}"
    );
}

#[test]
fn inertia_starter_scaffolds_console_binary_and_commands_dir() {
    let tmp = TempDir::new().unwrap();
    run_new(tmp.path(), "smoke-inertia", &["--frontend", "svelte"]);
    let project = tmp.path().join("smoke-inertia");

    let console = project.join("src/bin/console.rs");
    assert!(console.exists(), "console binary written");
    let console_src = read(&console);
    assert!(
        console_src.contains("suprnova::console::dispatch_argv_with_init"),
        "console uses the lazy-bootstrap form so --help / --version skip DB init"
    );
    assert!(
        console_src.contains("suprnova::console::set_version(env!(\"CARGO_PKG_VERSION\"))"),
        "console registers the user's package version so --version works"
    );
    assert!(console_src.contains("smoke_inertia::bootstrap::register"));
    assert!(
        console_src.contains("Err(error) => ExitCode::from(error.exit_code())"),
        "console ends with the exit code a command chose (PAR-177)"
    );
    // Two properties in one line, both load-bearing: the entry point is
    // `#[suprnova::main]` so `.env` loads before the runtime exists
    // (SEC-06), and the flavor stays single-threaded because a one-shot
    // console command has nothing to gain from a worker pool.
    assert!(
        console_src.contains("suprnova::main(flavor = \"current_thread\")"),
        "console must use #[suprnova::main(flavor = \"current_thread\")]"
    );

    let commands_mod = project.join("src/commands/mod.rs");
    assert!(commands_mod.exists(), "commands stub written");

    let cargo = read(project.join("Cargo.toml"));
    assert!(
        cargo.contains("name = \"console\""),
        "Cargo.toml declares the console [[bin]]: {cargo}"
    );
    assert!(cargo.contains("path = \"src/bin/console.rs\""));
    assert_production_build_shape(&cargo);

    let lib = read(project.join("src/lib.rs"));
    assert!(
        lib.contains("pub mod commands;"),
        "lib.rs declares the commands module"
    );
}

#[test]
fn api_starter_scaffolds_console_binary_and_commands_dir() {
    let tmp = TempDir::new().unwrap();
    run_new(tmp.path(), "smoke-api", &["--api"]);
    let project = tmp.path().join("smoke-api");

    let console = project.join("src/bin/console.rs");
    assert!(console.exists(), "api console binary written");
    let console_src = read(&console);
    assert!(
        console_src.contains("suprnova::console::dispatch_argv_with_init"),
        "api console uses the lazy-bootstrap form"
    );
    assert!(
        console_src.contains("suprnova::console::set_version(env!(\"CARGO_PKG_VERSION\"))"),
        "api console registers the user's package version"
    );
    assert!(console_src.contains("smoke_api::bootstrap::register"));
    assert!(
        console_src.contains("Err(error) => ExitCode::from(error.exit_code())"),
        "api console ends with the exit code a command chose (PAR-177)"
    );

    let commands_mod = project.join("src/commands/mod.rs");
    assert!(commands_mod.exists(), "api commands stub written");

    let cargo = read(project.join("Cargo.toml"));
    assert!(cargo.contains("name = \"console\""));
    assert!(cargo.contains("path = \"src/bin/console.rs\""));
    assert_production_build_shape(&cargo);

    let lib = read(project.join("src/lib.rs"));
    assert!(lib.contains("pub mod commands;"));
}

/// Asserts the two settings that match Laravel's MySQL schema are in the
/// new project's `Cargo.toml`, commented out, with the column types each
/// value needs and the per-field override beside them. The comment is all a
/// developer porting a Laravel application has to go on, so its parts are
/// pinned one by one; the manifest must still parse to no settings at all.
fn assert_laravel_settings_commented_out(cargo: &str) {
    for line in [
        "# [package.metadata.suprnova.model]",
        "# datetime_cast = \"native\"",
        "# [package.metadata.suprnova.schema]",
        "# unsigned_ids = false",
    ] {
        assert!(
            cargo.lines().any(|l| l.trim() == line),
            "Cargo.toml carries `{line}`: {cargo}"
        );
    }
    for words in [
        "Laravel's MySQL schema",
        "\"naive\"",
        "timestamp with time zone",
        "TIMESTAMP",
        "BIGINT UNSIGNED",
        "casts = {",
    ] {
        assert!(
            cargo.contains(words),
            "the comment mentions `{words}`: {cargo}"
        );
    }
    let manifest: toml::Table = cargo.parse().expect("the scaffolded Cargo.toml parses");
    let metadata = manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("suprnova"));
    assert!(
        metadata.is_none(),
        "the settings stay commented out, so nothing changes until a developer opts in"
    );
}

#[test]
fn new_scaffolds_carry_the_laravel_settings_commented_out() {
    let tmp = TempDir::new().unwrap();
    run_new(tmp.path(), "smoke-laravel", &["--frontend", "svelte"]);
    assert_laravel_settings_commented_out(&read(tmp.path().join("smoke-laravel/Cargo.toml")));
    run_new(tmp.path(), "smoke-laravel-api", &["--api"]);
    assert_laravel_settings_commented_out(&read(tmp.path().join("smoke-laravel-api/Cargo.toml")));
}
