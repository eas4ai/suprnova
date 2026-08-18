//! T15 — `suprnova serve`: crash respawn with backoff, `--no-restart`,
//! `--timestamps`, `--json` NDJSON events, and the `Suprnova.toml`
//! extra-process registry.
//!
//! Hermetic: `--frontend-only` skips the `cargo-watch` bootstrap entirely
//! (`ensure_cargo_watch` only runs without it), and a pre-created
//! `frontend/node_modules` makes `ensure_npm_dependencies` skip the real
//! `npm install`. `npm` itself is a PATH-shimmed shell script — the same
//! pattern `migrate_fresh_gate.rs` uses for `cargo`: `serve.rs` spawns
//! `Command::new("npm")` with no absolute path, so a shim earlier on
//! `PATH` is picked up transparently. The original three tests redirect
//! combined stdout/stderr to one file, since they only assert on
//! substrings; the two `--json` tests below redirect stdout and stderr
//! to *separate* files instead, since they assert every stdout line
//! parses as JSON and a stray stderr diagnostic sharing the file would
//! break that. Either way, output goes to a file rather than a pipe, so
//! nothing has to drain a pipe concurrently to avoid a deadlock once its
//! buffer fills.

#![cfg(unix)]

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

/// A `--frontend-only`-shaped project: just enough for
/// `validate_suprnova_project` to pass, plus a fixture-local `PATH`.
struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().expect("create tempdir");
        // Present ⇒ ensure_npm_dependencies skips the real `npm install`.
        fs::create_dir_all(dir.path().join("frontend/node_modules"))
            .expect("mkdir frontend/node_modules");
        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn bin_dir(&self) -> PathBuf {
        self.root().join("fakebin")
    }

    /// Write an executable `#!/bin/sh` script named `name` under the
    /// fixture's `fakebin/`. Returns its absolute path.
    fn shim(&self, name: &str, body: &str) -> PathBuf {
        fs::create_dir_all(self.bin_dir()).expect("mkdir fakebin");
        let path = self.bin_dir().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write shim");
        let mut perms = fs::metadata(&path).expect("stat shim").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod shim");
        path
    }

    fn write_suprnova_toml(&self, body: &str) {
        fs::write(self.root().join("Suprnova.toml"), body).expect("write Suprnova.toml");
    }

    /// Spawn `suprnova serve --frontend-only <extra_args>` with combined
    /// stdout/stderr redirected to a file and `PATH` pointed at the
    /// fixture's shim directory first.
    fn spawn_serve(&self, extra_args: &[&str]) -> (Child, PathBuf) {
        let out_path = self.root().join("serve.out");
        let out_file = fs::File::create(&out_path).expect("create serve.out");
        let err_file = out_file.try_clone().expect("clone serve.out handle");
        let path = format!(
            "{}:{}",
            self.bin_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let child = Command::new(BIN)
            .arg("serve")
            .arg("--frontend-only")
            .args(extra_args)
            .current_dir(self.root())
            .env("PATH", path)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .expect("spawn suprnova serve");
        (child, out_path)
    }

    /// Like `spawn_serve`, but stdout and stderr go to *separate* files
    /// instead of one combined stream. `--json` mode's contract is about
    /// stdout specifically ("one JSON object per line, nothing else");
    /// mixing in stderr's `ui::warning`/`ui::error` diagnostics would
    /// make a test that asserts "every stdout line is JSON" fail for a
    /// reason unrelated to what it's checking.
    fn spawn_serve_split(&self, extra_args: &[&str]) -> (Child, PathBuf, PathBuf) {
        let out_path = self.root().join("serve.stdout");
        let err_path = self.root().join("serve.stderr");
        let out_file = fs::File::create(&out_path).expect("create serve.stdout");
        let err_file = fs::File::create(&err_path).expect("create serve.stderr");
        let path = format!(
            "{}:{}",
            self.bin_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let child = Command::new(BIN)
            .arg("serve")
            .arg("--frontend-only")
            .args(extra_args)
            .current_dir(self.root())
            .env("PATH", path)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .expect("spawn suprnova serve");
        (child, out_path, err_path)
    }
}

/// Poll `cond` every 20ms until it's true or `timeout` elapses.
fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn looks_like_hh_mm_ss(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 8
        && b[2] == b':'
        && b[5] == b':'
        && b[0..2].iter().all(u8::is_ascii_digit)
        && b[3..5].iter().all(u8::is_ascii_digit)
        && b[6..8].iter().all(u8::is_ascii_digit)
}

#[test]
fn crashed_dev_process_is_respawned_session_stays_alive_and_lines_are_timestamped() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!(
            "echo shim-alive\necho x >> \"{}\"\nexit 1",
            counter.display()
        ),
    );

    let (mut child, out_path) = fx.spawn_serve(&["--timestamps"]);

    // 200 + 400 + 800ms of cumulative backoff clears 3 respawns well
    // inside this window.
    std::thread::sleep(Duration::from_millis(1800));

    assert_eq!(
        child.try_wait().expect("try_wait"),
        None,
        "the session must still be running despite the crash loop"
    );

    let invocations = fs::read_to_string(&counter).unwrap_or_default();
    let count = invocations.lines().count();
    assert!(
        count >= 3,
        "expected at least 3 respawns in 1.8s of 200/400/800ms backoff, got {count}"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    let line = output
        .lines()
        .find(|l| l.contains("shim-alive"))
        .unwrap_or_else(|| panic!("no shim-alive line in output: {output}"));
    let stamp = line.split_whitespace().next().unwrap_or("");
    assert!(
        looks_like_hh_mm_ss(stamp),
        "expected an HH:MM:SS timestamp at the start of {line:?}"
    );
    assert!(line.contains("[frontend]"), "{line}");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn no_restart_flag_tears_the_session_down_on_a_single_crash() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!("echo x >> \"{}\"\nexit 1", counter.display()),
    );

    let (mut child, _out_path) = fx.spawn_serve(&["--no-restart"]);

    let exited = wait_until(Duration::from_secs(3), || {
        matches!(child.try_wait(), Ok(Some(_)))
    });
    assert!(
        exited,
        "--no-restart must tear the session down after one crash"
    );

    let invocations = fs::read_to_string(&counter).unwrap_or_default();
    assert_eq!(
        invocations.lines().count(),
        1,
        "must not respawn under --no-restart"
    );
}

#[test]
fn suprnova_toml_process_registry_entry_is_spawned_with_its_own_prefix() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!("echo x >> \"{}\"\nexit 1", counter.display()),
    );
    let worker = fx.shim("queue-worker", "echo queue-worker-alive\nexit 0");
    fx.write_suprnova_toml(&format!(
        "[[serve.process]]\nname = \"queue\"\ncommand = \"{}\"\nargs = []\ncolor = \"green\"\n",
        worker.display()
    ));

    let (mut child, out_path) = fx.spawn_serve(&["--no-restart"]);

    wait_until(Duration::from_secs(3), || {
        matches!(child.try_wait(), Ok(Some(_)))
    });
    let _ = child.wait();

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    assert!(
        output.contains("[queue] queue-worker-alive"),
        "output: {output}"
    );
}

#[test]
fn json_mode_emits_valid_ndjson_across_a_start_output_exit_restart_lifecycle() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!(
            "echo shim-alive\necho x >> \"{}\"\nexit 1",
            counter.display()
        ),
    );

    let (mut child, out_path, _err_path) = fx.spawn_serve_split(&["--json"]);

    // 200 + 400ms of cumulative backoff clears at least 2 respawns well
    // inside this window - enough to observe every event kind below.
    std::thread::sleep(Duration::from_millis(1200));

    assert_eq!(
        child.try_wait().expect("try_wait"),
        None,
        "the session must still be running despite the crash loop"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    assert!(!output.is_empty(), "--json mode must still produce output");

    let mut seen_types = HashSet::new();
    for line in output.lines().filter(|l| !l.is_empty()) {
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line {line:?} is not valid JSON: {e}"));
        let ty = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("line {line:?} has no \"type\" field"))
            .to_string();
        assert!(
            value.get("ts").is_some(),
            "line {line:?} is missing its \"ts\" field"
        );
        seen_types.insert(ty);
    }

    for expected in ["started", "output", "exited", "restart_scheduled"] {
        assert!(
            seen_types.contains(expected),
            "expected a {expected:?} event among types {seen_types:?}"
        );
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn json_mode_suppresses_the_prefixed_human_output() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!(
            "echo shim-alive\necho x >> \"{}\"\nexit 1",
            counter.display()
        ),
    );

    // --timestamps alongside --json proves the documented interaction:
    // --json wins, --timestamps is a harmless no-op, not an error.
    let (mut child, out_path, _err_path) = fx.spawn_serve_split(&["--json", "--timestamps"]);

    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        child.try_wait().expect("try_wait"),
        None,
        "the session must still be running"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    assert!(
        !output.contains("[frontend]"),
        "the [name]-prefixed format must not appear in --json mode: {output}"
    );
    assert!(!output.is_empty(), "--json mode must still produce output");
    for line in output.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("line {line:?} is not valid JSON: {e}"));
    }

    let _ = child.kill();
    let _ = child.wait();
}
