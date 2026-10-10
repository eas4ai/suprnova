//! A generated application's account flows, validation-error display and
//! static-file serving, proven by running the generated application.
//!
//! Scaffolds a Vue project, points it at the in-tree framework crate,
//! writes `tests/fixtures/scaffold/account_flows.rs` into it, and runs
//! `cargo test` there. That test drives the generated router in-process
//! over a real Hyper connection with mail captured in memory (see the
//! fixture for what it covers: GitHub issues #6, #7 and #8).
//!
//! `#[ignore]`d for the same reason as the compile checks in
//! `scaffold_snapshot.rs`: it builds the framework and the generated
//! application. Run it explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test scaffold_account_flows -- --ignored --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// The test that runs inside the generated project. `__PACKAGE__` is the
/// generated crate's name.
const ACCOUNT_FLOWS_TEST: &str = include_str!("fixtures/scaffold/account_flows.rs");

const PROJECT: &str = "account_flows";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("suprnova-cli sits inside the workspace")
        .to_path_buf()
}

/// A scratch directory under this package's own target directory. The
/// generated application builds its own `target/` inside it, far too many
/// files for a tmpfs `/tmp`.
fn scratch_dir() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir")
}

fn scaffold(tmp: &Path) -> PathBuf {
    let output = Command::new(BIN)
        .args([
            "new",
            PROJECT,
            "--no-interaction",
            "--no-git",
            "--frontend",
            "vue",
        ])
        .current_dir(tmp)
        .output()
        .expect("suprnova new");
    assert!(
        output.status.success(),
        "suprnova new failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    tmp.join(PROJECT)
}

/// Point every `suprnova = { git = ..., tag = ... }` line at the in-tree
/// framework, keeping the feature shape the template chose, and add the
/// HTTP crates the in-process client needs, pinned to the versions the
/// workspace lockfile resolves.
fn patch_manifest(project: &Path) {
    let manifest_path = project.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path).expect("read scaffolded Cargo.toml");
    let framework = workspace_root().join("framework");

    let mut rewritten = String::with_capacity(original.len() + 256);
    let mut replaced = 0;
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            let (before, rest) = line.split_once("git = ").expect("a git source");
            let (_, after_tag) = rest.split_once("tag = ").expect("a tag");
            let (_, tail) = after_tag[1..].split_once('"').expect("a closing quote");
            rewritten.push_str(&format!(
                "{before}path = \"{}\"{tail}\n",
                framework.display()
            ));
            replaced += 1;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert_eq!(
        replaced, 2,
        "the scaffold declares suprnova once as a dependency and once as a dev-dependency"
    );

    // The dev-dependency table is the manifest's last table, so appended
    // lines land in it. Assert that rather than assume it.
    let last_table = rewritten
        .lines()
        .rev()
        .find(|line| line.trim().starts_with('['))
        .expect("a table header");
    assert_eq!(
        last_table.trim(),
        "[dev-dependencies]",
        "the scaffold's manifest ends with its dev-dependencies"
    );
    if !rewritten.ends_with('\n') {
        rewritten.push('\n');
    }
    for (name, extra) in [
        ("bytes", ""),
        ("http-body-util", ""),
        ("hyper", r#", features = ["client", "server", "http1"]"#),
        ("hyper-util", r#", features = ["tokio"]"#),
    ] {
        rewritten.push_str(&format!(
            "{name} = {{ version = \"={}\"{extra} }}\n",
            locked_version(name)
        ));
    }
    // The project lives under this workspace's `target/`, so without a
    // workspace table of its own cargo would claim it as a member of the
    // workspace above it and refuse to build.
    rewritten.push_str("\n[workspace]\n");
    fs::write(&manifest_path, rewritten).expect("write patched Cargo.toml");
}

/// The single version the workspace lockfile resolves `name` to.
fn locked_version(name: &str) -> String {
    let lock = fs::read_to_string(workspace_root().join("Cargo.lock")).expect("read Cargo.lock");
    let header = format!("name = \"{name}\"\n");
    let versions: Vec<&str> = lock
        .split("[[package]]")
        .filter_map(|package| package.trim_start().strip_prefix(&header))
        .filter_map(|rest| rest.strip_prefix("version = \""))
        .filter_map(|rest| rest.split_once('"'))
        .map(|(version, _)| version)
        .collect();
    match versions.as_slice() {
        [only] => (*only).to_owned(),
        [] => panic!("Cargo.lock does not resolve `{name}`"),
        many => panic!("Cargo.lock resolves `{name}` to several versions: {many:?}"),
    }
}

/// The settings the scaffold wrote into `.env`, as `KEY=VALUE` pairs. The
/// test binary never loads `.env` itself; the harness hands the file's
/// settings over through the environment, then overrides the few a test
/// run has to control. Running under the scaffold's own settings is the
/// point: it is the configuration a user's fresh project starts with.
fn scaffolded_env(project: &Path) -> Vec<(String, String)> {
    let env = fs::read_to_string(project.join(".env")).expect("read scaffolded .env");
    let pairs: Vec<(String, String)> = env
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.trim_matches('"').to_owned()))
        .collect();
    assert!(
        pairs.iter().any(|(key, _)| key == "APP_KEY"),
        ".env carries APP_KEY"
    );
    pairs
}

#[test]
#[ignore = "runs the generated application's own tests; builds the framework and the app; slow"]
fn a_generated_application_verifies_email_resets_passwords_and_serves_its_assets() {
    let tmp = scratch_dir();
    let project = scaffold(tmp.path());
    patch_manifest(&project);

    let package = PROJECT;
    fs::create_dir_all(project.join("tests")).expect("create tests/");
    fs::write(
        project.join("tests/account_flows.rs"),
        ACCOUNT_FLOWS_TEST.replace("__PACKAGE__", package),
    )
    .expect("write the in-project test");

    // What `npm run build` would leave behind: a hashed bundle and Vite's
    // manifest, the one file under public/ that must not be served.
    let assets = project.join("public/assets");
    fs::create_dir_all(assets.join(".vite")).expect("create public/assets/.vite");
    fs::write(assets.join("app-1234.js"), "console.log('built')\n").expect("write asset");
    fs::write(assets.join(".vite/manifest.json"), "{}\n").expect("write manifest");

    let database = tmp.path().join("account_flows.sqlite");
    // A stable target directory outside the temp dir, so a second run
    // reuses the framework build instead of paying for it again.
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("account_flows_target");
    let output = Command::new(env!("CARGO"))
        .args(["test", "--test", "account_flows", "--", "--nocapture"])
        .current_dir(&project)
        .envs(scaffolded_env(&project))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0")
        .env("APP_ENV", "test")
        .env("APP_URL", "http://app.test")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", database.display()),
        )
        .env("MAIL_FROM", "ops@example.test")
        .env("MAIL_DRIVER", "log")
        .output()
        .expect("cargo test in the generated project");
    assert!(
        output.status.success(),
        "the generated application's account-flow test failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test account_flows_validation_errors_and_static_files ... ok"),
        "the in-project test must have run and passed:\n{stdout}"
    );
}
