//! `suprnova migrate:fresh` drops every table in the database. In production
//! it must refuse unless the operator passes `--force` *and* types the
//! environment name at an interactive prompt.
//!
//! These tests assert on an observable seam rather than on the printed
//! message: a fake `cargo` is placed first on `PATH` that touches a sentinel
//! file when it runs. The migrator is spawned as `cargo run -- migrate:fresh`,
//! so the sentinel existing means the drop was really handed off. A refusal
//! that still printed the right words while spawning the migrator would pass
//! a message-only assertion and fail these.
//!
//! Teeth: with the guard removed, `sentinel exists == true` in both refusal
//! tests below.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// A project directory with a migrations dir (so the command gets past its
/// first guard) and a `cargo` shim on PATH that records being called.
struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().expect("create tempdir");
        fs::create_dir_all(dir.path().join("src/migrations")).expect("mkdir src/migrations");

        let bin = dir.path().join("fakebin");
        fs::create_dir_all(&bin).expect("mkdir fakebin");
        let shim = bin.join("cargo");
        fs::write(
            &shim,
            "#!/bin/sh\ntouch \"$MIGRATE_FRESH_SENTINEL\"\nexit 0\n",
        )
        .expect("write cargo shim");
        let mut perms = fs::metadata(&shim).expect("stat shim").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim, perms).expect("chmod shim");

        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn sentinel(&self) -> std::path::PathBuf {
        self.dir.path().join("migrator-was-spawned")
    }

    fn run(&self, app_env: &str, args: &[&str]) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.path().join("fakebin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        Command::new(BIN)
            .arg("migrate:fresh")
            .args(args)
            .env("APP_ENV", app_env)
            .env("PATH", path)
            .env("MIGRATE_FRESH_SENTINEL", self.sentinel())
            .current_dir(self.dir.path())
            .output()
            .expect("spawn suprnova binary")
    }
}

#[test]
fn production_without_force_refuses_and_never_spawns_the_migrator() {
    let fx = Fixture::new();
    let out = fx.run("production", &[]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "production without --force must exit 1; output: {text}"
    );
    assert!(
        !fx.sentinel().exists(),
        "the migrator was spawned on an unconfirmed production path; output: {text}"
    );
    assert!(
        text.contains("--force"),
        "the refusal must say what is missing; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn production_with_force_but_no_terminal_refuses_and_never_spawns_the_migrator() {
    // `Command::output()` gives the child a non-terminal stdin, which is
    // exactly the CI/deploy-script shape the guard exists for.
    let fx = Fixture::new();
    let out = fx.run("production", &["--force"]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "--force alone must not be enough; output: {text}"
    );
    assert!(
        !fx.sentinel().exists(),
        "the migrator was spawned without a typed confirmation; output: {text}"
    );
    assert!(
        text.contains("terminal"),
        "the refusal must explain the TTY requirement; got: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}

#[test]
fn a_piped_confirmation_is_not_accepted_as_a_typed_one() {
    // The point of the TTY requirement: `echo production | ... --force` in a
    // script must not satisfy the prompt.
    let fx = Fixture::new();
    let path = format!(
        "{}:{}",
        fx.root().join("fakebin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "echo production | '{}' migrate:fresh --force",
            BIN.replace('\'', r"'\''")
        ))
        .env("APP_ENV", "production")
        .env("PATH", path)
        .env("MIGRATE_FRESH_SENTINEL", fx.sentinel())
        .current_dir(fx.root())
        .output()
        .expect("spawn shell");
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a piped answer must not confirm; output: {text}"
    );
    assert!(
        !fx.sentinel().exists(),
        "a piped answer spawned the migrator; output: {text}"
    );
}

/// The seam has to be able to report "yes it ran" too, or the assertions
/// above would pass against a command that never spawns anything.
#[test]
fn a_non_production_environment_still_spawns_the_migrator() {
    let fx = Fixture::new();
    let out = fx.run("local", &[]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(0),
        "local must run normally; output: {text}"
    );
    assert!(
        fx.sentinel().exists(),
        "the migrator should have been spawned for APP_ENV=local; output: {text}"
    );
}

/// The pre-existing "is this even a project" guard still fires first, and
/// still without spawning anything.
#[test]
fn a_missing_migrations_directory_refuses_before_anything_runs() {
    let fx = Fixture::new();
    fs::remove_dir_all(fx.root().join("src/migrations")).expect("remove migrations dir");
    let out = fx.run("local", &[]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "no migrations directory must exit 1; output: {text}"
    );
    assert!(
        !fx.sentinel().exists(),
        "nothing should be spawned without migrations; output: {text}"
    );
    assert!(!text.contains("panicked"), "must NOT panic; got: {text}");
}
