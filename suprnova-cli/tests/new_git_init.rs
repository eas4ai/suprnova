#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

const FAILING_GIT: &str = r#"#!/bin/sh
{
    printf 'cwd=%s\n' "$PWD"
    printf 'argc=%s\n' "$#"
    for arg in "$@"; do
        printf 'arg=%s\n' "$arg"
    done
} > "$GIT_SHIM_LOG"
printf '%s\n' 'fixture git failure' >&2
exit 42
"#;

const SUCCESSFUL_GIT: &str = r#"#!/bin/sh
{
    printf 'cwd=%s\n' "$PWD"
    printf 'argc=%s\n' "$#"
    for arg in "$@"; do
        printf 'arg=%s\n' "$arg"
    done
} > "$GIT_SHIM_LOG"
exit 0
"#;

struct Fixture {
    root: TempDir,
    bin_dir: PathBuf,
    git_log: PathBuf,
}

impl Fixture {
    fn new(git_script: &str) -> Self {
        let root = tempdir().expect("create fixture root");
        let bin_dir = root.path().join("fakebin");
        fs::create_dir_all(&bin_dir).expect("create fakebin");
        let git = bin_dir.join("git");
        fs::write(&git, git_script).expect("write git shim");
        let mut permissions = fs::metadata(&git).expect("stat git shim").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&git, permissions).expect("make git shim executable");
        let git_log = root.path().join("git-invocation.log");

        Self {
            root,
            bin_dir,
            git_log,
        }
    }

    fn run(&self, name: &str, api: bool, no_git: bool) -> Output {
        let mut path = OsString::from(self.bin_dir.as_os_str());
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        let mut command = Command::new(BIN);
        command.args(["new", name, "--no-interaction"]);
        if api {
            command.arg("--api");
        }
        if no_git {
            command.arg("--no-git");
        }
        command
            .current_dir(self.root.path())
            .env("PATH", path)
            .env("GIT_SHIM_LOG", &self.git_log)
            .output()
            .expect("run suprnova new")
    }

    fn project(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
}

fn combined(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[test]
fn failed_git_init_is_reported_for_full_and_api_scaffolds() {
    for (name, api) in [("full-app", false), ("api-app", true)] {
        let fixture = Fixture::new(FAILING_GIT);
        let output = fixture.run(name, api, false);
        let text = combined(&output);

        assert_eq!(
            output.status.code(),
            Some(1),
            "a failed git init must fail the command; output: {text}"
        );
        assert!(
            text.contains("Failed to initialize git repository"),
            "the error must identify the failed step; output: {text}"
        );
        assert!(
            text.contains("`git init` exited with 42"),
            "the error must include the child exit code; output: {text}"
        );
        assert!(
            text.contains("fixture git failure"),
            "the error must retain useful stderr; output: {text}"
        );
        assert!(
            !text.contains("Initialized git repository"),
            "the failed step must not get a success banner; output: {text}"
        );
        assert!(
            !text.contains("Ready to go!"),
            "the command must not claim completion; output: {text}"
        );
        assert!(
            fixture.project(name).join("Cargo.toml").exists(),
            "the generated project must remain available after init failure"
        );
    }
}

#[test]
fn no_git_skips_the_git_process_for_full_and_api_scaffolds() {
    for (name, api) in [("full-no-git", false), ("api-no-git", true)] {
        let fixture = Fixture::new(FAILING_GIT);
        let output = fixture.run(name, api, true);
        let text = combined(&output);

        assert_eq!(output.status.code(), Some(0), "output: {text}");
        assert!(text.contains("Ready to go!"), "output: {text}");
        assert!(
            !fixture.git_log.exists(),
            "--no-git must not invoke the git shim"
        );
    }
}

#[test]
fn successful_git_init_uses_the_generated_project_as_its_working_directory() {
    let fixture = Fixture::new(SUCCESSFUL_GIT);
    let name = "successful-app";
    let output = fixture.run(name, false, false);
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(0), "output: {text}");
    assert!(
        text.contains("Initialized git repository"),
        "output: {text}"
    );
    assert!(text.contains("Ready to go!"), "output: {text}");

    let expected = format!(
        "cwd={}\nargc=1\narg=init\n",
        fixture.project(name).display()
    );
    assert_eq!(
        fs::read_to_string(&fixture.git_log).expect("read git invocation"),
        expected
    );
}
