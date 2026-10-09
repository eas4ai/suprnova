//! `--help` must print help and run nothing.
//!
//! The shipped CLI disabled clap's help flag and declared `--help` as an
//! ordinary global `bool`, so `suprnova <subcommand> --help` parsed the
//! flag, discarded it, and ran the subcommand. `APP_ENV` defaults to
//! `local`, where `migrate:fresh` drops every table without a
//! confirmation, so `suprnova migrate:fresh --help` destroyed a
//! developer's database; `suprnova generate-types --help` rewrote the
//! generated types file and exited 0.
//!
//! Every subcommand is covered by the enumerated unit test in
//! `src/main.rs`, which reads the subcommand list back from clap and
//! proves a help request never yields parsed arguments to dispatch. These
//! tests are the end-to-end half: the real binary, the real exit code, and
//! an observable seam for "ran nothing" rather than a message-only
//! assertion.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

/// Run the CLI in `dir` with `APP_ENV=local` - the default environment,
/// and the one in which the destructive commands do not stop to ask.
fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("APP_ENV", "local")
        .current_dir(dir)
        .output()
        .expect("spawn suprnova binary")
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).expect("read tempdir").next().is_none()
}

/// Ask for a subcommand's help in an empty directory: it must exit 0, print
/// that subcommand's own usage, and leave the directory untouched.
fn assert_help_only(subcommand: &str, flag: &str) {
    let tmp = tempdir().expect("create tempdir");
    let out = run_in(tmp.path(), &[subcommand, flag]);
    let text = combined(&out);

    assert_eq!(
        out.status.code(),
        Some(0),
        "`suprnova {subcommand} {flag}` must exit 0; output: {text}"
    );
    assert!(
        text.contains(&format!("Usage: suprnova {subcommand}")),
        "`suprnova {subcommand} {flag}` must print that subcommand's help; \
         output: {text}"
    );
    assert!(
        is_empty(tmp.path()),
        "`suprnova {subcommand} {flag}` wrote to the working directory"
    );
}

/// The benign form the application team hit: `generate-types --help`
/// regenerated their types file and exited 0.
#[test]
fn generate_types_help_prints_help_and_writes_nothing() {
    for flag in ["--help", "-h"] {
        assert_help_only("generate-types", flag);
    }
}

/// A subcommand with a required positional used to fail its help request
/// with a missing-argument error instead, because the required argument was
/// validated before anything looked at the flag.
#[test]
fn a_subcommand_with_a_required_argument_prints_help() {
    for flag in ["--help", "-h"] {
        assert_help_only("make:controller", flag);
    }
}

/// The top level keeps its curated screen on both spellings.
#[test]
fn the_top_level_help_flags_print_the_curated_screen() {
    let tmp = tempdir().expect("create tempdir");
    for flag in ["--help", "-h"] {
        let out = run_in(tmp.path(), &[flag]);
        let text = combined(&out);
        assert_eq!(
            out.status.code(),
            Some(0),
            "`suprnova {flag}` must exit 0; output: {text}"
        );
        assert!(
            text.contains("USAGE:") && text.contains("live:make"),
            "`suprnova {flag}` must print the curated screen; output: {text}"
        );
    }
}

/// `migrate:fresh` is the reason this is a release blocker, so it gets the
/// same observable seam `migrate_fresh_gate.rs` uses: a fake `cargo` first
/// on `PATH` that touches a sentinel when the migrator is really spawned.
/// A help request that still dropped the tables would print the right words
/// and fail here.
#[cfg(unix)]
mod migrate_fresh {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

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

        fn sentinel(&self) -> std::path::PathBuf {
            self.dir.path().join("migrator-was-spawned")
        }

        fn run(&self, args: &[&str]) -> Output {
            let path = format!(
                "{}:{}",
                self.dir.path().join("fakebin").display(),
                std::env::var("PATH").unwrap_or_default()
            );
            Command::new(BIN)
                .arg("migrate:fresh")
                .args(args)
                .env("APP_ENV", "local")
                .env("PATH", path)
                .env("MIGRATE_FRESH_SENTINEL", self.sentinel())
                .current_dir(self.dir.path())
                .output()
                .expect("spawn suprnova binary")
        }
    }

    #[test]
    fn a_help_request_prints_help_and_never_spawns_the_migrator() {
        for flag in ["--help", "-h"] {
            let fx = Fixture::new();
            let out = fx.run(&[flag]);
            let text = combined(&out);

            assert_eq!(
                out.status.code(),
                Some(0),
                "`suprnova migrate:fresh {flag}` must exit 0; output: {text}"
            );
            assert!(
                text.contains("Usage: suprnova migrate:fresh"),
                "`suprnova migrate:fresh {flag}` must print its help; output: {text}"
            );
            assert!(
                !fx.sentinel().exists(),
                "`suprnova migrate:fresh {flag}` dropped the tables; output: {text}"
            );
        }
    }

    /// Teeth for the test above: the seam has to be able to report "yes it
    /// ran", or an absent sentinel would prove nothing.
    #[test]
    fn without_a_help_flag_the_migrator_is_still_spawned() {
        let fx = Fixture::new();
        let out = fx.run(&[]);
        let text = combined(&out);

        assert_eq!(
            out.status.code(),
            Some(0),
            "`suprnova migrate:fresh` must run normally in a local environment; \
             output: {text}"
        );
        assert!(
            fx.sentinel().exists(),
            "the migrator should have been spawned; output: {text}"
        );
    }
}
