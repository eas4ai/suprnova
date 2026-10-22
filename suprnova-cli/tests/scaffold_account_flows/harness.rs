//! Run a test inside a generated Vue application: scaffold it, point it at
//! the in-tree framework, and run one of its own test binaries under the
//! settings `suprnova new` wrote into its `.env`.
//!
//! `scaffold_account_flows.rs` and the agents case of `scaffold_snapshot.rs`
//! both declare this module with a `#[path]` line. Both build into one
//! target directory, so the framework builds once for both.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The CLI binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// The workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("suprnova-cli sits inside the workspace")
        .to_path_buf()
}

/// A scratch directory under this package's own target directory. The
/// generated application builds its own `target/` inside it, far too many
/// files for a tmpfs `/tmp`.
pub fn scratch_dir() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir")
}

/// Scaffold a Vue project named `project` into `tmp`; its root.
pub fn scaffold(tmp: &Path, project: &str) -> PathBuf {
    let output = Command::new(BIN)
        .args([
            "new",
            project,
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
    tmp.join(project)
}

/// Point every `suprnova = { git = ..., tag = ... }` line, or its `rev =`
/// form, at the in-tree framework, keeping the feature shape the template
/// chose, and add the HTTP crates the in-process client needs, pinned to
/// the versions the workspace lockfile resolves.
pub fn patch_manifest(project: &Path) {
    let manifest_path = project.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path).expect("read scaffolded Cargo.toml");
    let framework = workspace_root().join("framework");

    let mut rewritten = String::with_capacity(original.len() + 256);
    let mut replaced = 0;
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            let (before, rest) = line.split_once("git = ").expect("a git source");
            let (_, after_pin) = rest
                .split_once("tag = ")
                .or_else(|| rest.split_once("rev = "))
                .expect("a tag or rev pin");
            let (_, tail) = after_pin[1..].split_once('"').expect("a closing quote");
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
/// point: it is the configuration a user's fresh project starts with, its
/// agent auth signing key included.
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

/// Write `source`, with `__PACKAGE__` replaced by the project's crate name,
/// as the project's test `tests/<test>.rs`, and run it with `cargo test`
/// under the scaffold's settings, with `APP_URL` set to `app_url`, on a
/// SQLite database of its own in `tmp`. The run builds into one target
/// directory outside `tmp`, so a second run, or the other harness, reuses
/// the framework build instead of paying for it again.
pub fn run_project_test(
    tmp: &Path,
    project: &Path,
    test: &str,
    source: &str,
    app_url: &str,
) -> Output {
    let package = project
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the project's directory names its crate");
    fs::create_dir_all(project.join("tests")).expect("create tests/");
    fs::write(
        project.join(format!("tests/{test}.rs")),
        source.replace("__PACKAGE__", package),
    )
    .expect("write the in-project test");

    let database = tmp.join(format!("{test}.sqlite"));
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("account_flows_target");
    Command::new(env!("CARGO"))
        .args(["test", "--test", test, "--", "--nocapture"])
        .current_dir(project)
        .envs(scaffolded_env(project))
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0")
        .env("APP_ENV", "test")
        .env("APP_URL", app_url)
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", database.display()),
        )
        .env("MAIL_FROM", "ops@example.test")
        .env("MAIL_DRIVER", "log")
        .output()
        .expect("cargo test in the generated project")
}
