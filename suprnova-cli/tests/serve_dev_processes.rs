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
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill, killpg};
use nix::unistd::Pid;

use serde_json::Value;
use tempfile::{TempDir, tempdir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

const CARGO_WATCH_SHIM: &str = r#"if [ "$1" = "watch" ] && [ "$2" = "--version" ]; then
  exit 0
fi
trap 'exit 0' TERM INT
printf 'backend-shim-alive\n'
sleep 10"#;

const CARGO_WATCH_DESCENDANT_SHIM: &str = r#"if [ "$1" = "watch" ] && [ "$2" = "--version" ]; then
  exit 0
fi
trap 'exit 0' TERM INT
printf 'backend-shim-alive\n'
sleep 10 &
descendant=$!
printf '%s\n' "$descendant" > cargo-descendant.pid
wait "$descendant""#;

struct ProcessGroupChild {
    child: Option<Child>,
}

impl ProcessGroupChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn process_group_id(&self) -> u32 {
        self.id()
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("child present").id()
    }

    fn terminate(&mut self) {
        self.terminate_with_signal(signal_group);
    }

    fn terminate_with_signal(&mut self, mut signal_group: impl FnMut(u32, Signal) -> bool) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let pgid = child.id();

        let term_signaled = signal_group(pgid, Signal::SIGTERM);
        let mut member_fallback_required = !term_signaled;

        if term_signaled {
            let term_stopped = wait_until(Duration::from_secs(2), || {
                !process_group_has_live_members(pgid)
            });

            if !term_stopped {
                let kill_signaled = signal_group(pgid, Signal::SIGKILL);
                let kill_stopped = kill_signaled
                    && wait_until(Duration::from_secs(2), || {
                        !process_group_has_live_members(pgid)
                    });
                member_fallback_required = !kill_stopped;
            }
        }

        if member_fallback_required {
            kill_live_process_group_members(pgid);
        }
        if member_fallback_required || process_is_live(pgid) {
            let _ = child.kill();
        }

        let _ = wait_until(Duration::from_secs(2), || {
            !process_group_has_live_members(pgid)
        });
        let _ = child.wait();
        let _ = wait_until(Duration::from_secs(2), || !process_group_exists(pgid));
    }
}

impl Drop for ProcessGroupChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Clone, Copy)]
struct ProcessGroupMember {
    pid: u32,
    zombie: bool,
}

fn signal_group(pgid: u32, signal: Signal) -> bool {
    let Ok(pgid) = i32::try_from(pgid) else {
        return false;
    };

    killpg(Pid::from_raw(pgid), signal).is_ok()
}

fn kill_live_process_group_members(pgid: u32) {
    let Some(members) = process_group_members(pgid) else {
        return;
    };

    for member in members.into_iter().filter(|member| !member.zombie) {
        let Ok(pid) = i32::try_from(member.pid) else {
            continue;
        };
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
}

fn process_group_exists(pgid: u32) -> bool {
    process_group_has_members(pgid, true)
}

fn process_group_has_live_members(pgid: u32) -> bool {
    process_group_has_members(pgid, false)
}

fn process_group_has_members(pgid: u32, include_zombies: bool) -> bool {
    process_group_members(pgid).is_none_or(|members| {
        members
            .iter()
            .any(|member| include_zombies || !member.zombie)
    })
}

fn process_group_members(pgid: u32) -> Option<Vec<ProcessGroupMember>> {
    let output = Command::new("ps")
        .args(["-eo", "pid=,pgid=,stat="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let processes = std::str::from_utf8(&output.stdout).ok()?;
    let mut members = Vec::new();

    for line in processes.lines() {
        let mut fields = line.split_whitespace();
        let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Some(process_pgid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Some(status) = fields.next() else {
            continue;
        };
        if process_pgid == pgid {
            members.push(ProcessGroupMember {
                pid,
                zombie: status.starts_with('Z'),
            });
        }
    }

    Some(members)
}

fn process_state(pid: u32) -> Option<u8> {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    output
        .stdout
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
}

fn process_is_live(pid: u32) -> bool {
    process_state(pid).is_some_and(|state| state != b'Z')
}

fn process_is_zombie(pid: u32) -> bool {
    process_state(pid) == Some(b'Z')
}

fn process_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };

    kill(Pid::from_raw(pid), None).is_ok()
}

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

    /// Write a minimal `Cargo.toml` with `[package].name = package_name`,
    /// which is all `get_package_name()`
    /// (`cargo_meta::package_name_from_content`) needs, plus an empty
    /// `src/`, so `validate_suprnova_project` and the type watcher's
    /// `watcher.watch("src", ...)` both succeed without `!frontend_only`.
    fn write_backend_project(&self, package_name: &str) {
        fs::write(
            self.root().join("Cargo.toml"),
            format!("[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\n"),
        )
        .expect("write Cargo.toml");
        fs::create_dir_all(self.root().join("src")).expect("mkdir src");
    }

    /// Spawn `suprnova serve --frontend-only <extra_args>` with combined
    /// stdout/stderr redirected to a file and `PATH` pointed at the
    /// fixture's shim directory first.
    fn spawn_serve(&self, extra_args: &[&str]) -> (ProcessGroupChild, PathBuf) {
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
            .process_group(0)
            .spawn()
            .expect("spawn suprnova serve");
        (ProcessGroupChild::new(child), out_path)
    }

    /// Like `spawn_serve`, but stdout and stderr go to *separate* files
    /// instead of one combined stream. `--json` mode's contract is about
    /// stdout specifically ("one JSON object per line, nothing else");
    /// mixing in stderr's `ui::warning`/`ui::error` diagnostics would
    /// make a test that asserts "every stdout line is JSON" fail for a
    /// reason unrelated to what it's checking.
    fn spawn_serve_split(&self, extra_args: &[&str]) -> (ProcessGroupChild, PathBuf, PathBuf) {
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
            .process_group(0)
            .spawn()
            .expect("spawn suprnova serve");
        (ProcessGroupChild::new(child), out_path, err_path)
    }

    /// Like `spawn_serve_split`, but doesn't force `--frontend-only` - the
    /// caller passes whichever flags it needs (typically `--backend-only`
    /// so no real `npm`/`frontend/` is required). This is the only way to
    /// exercise the `!frontend_only` path, which is what gates the
    /// TypeScript-regeneration file watcher on (`!skip_types &&
    /// !frontend_only` in `run()`) - every other test in this suite runs
    /// `--frontend-only` specifically to dodge `ensure_cargo_watch()`'s
    /// real `cargo install`, which also means none of them exercise the
    /// watcher at all.
    fn spawn_serve_split_full(&self, extra_args: &[&str]) -> (ProcessGroupChild, PathBuf, PathBuf) {
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
            .args(extra_args)
            .current_dir(self.root())
            .env("PATH", path)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .process_group(0)
            .spawn()
            .expect("spawn suprnova serve");
        (ProcessGroupChild::new(child), out_path, err_path)
    }

    fn spawn_serve_socket_full(&self, extra_args: &[&str]) -> (ProcessGroupChild, UnixStream) {
        let (output, child_output) = UnixStream::pair().expect("create output socket pair");
        let child_error = child_output
            .try_clone()
            .expect("clone output socket for stderr");
        let child_output: OwnedFd = child_output.into();
        let child_error: OwnedFd = child_error.into();
        let path = format!(
            "{}:{}",
            self.bin_dir().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let child = Command::new(BIN)
            .arg("serve")
            .args(extra_args)
            .current_dir(self.root())
            .env("PATH", path)
            .stdout(Stdio::from(child_output))
            .stderr(Stdio::from(child_error))
            .process_group(0)
            .spawn()
            .expect("spawn suprnova serve");
        (ProcessGroupChild::new(child), output)
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

    assert!(
        process_is_live(child.id()),
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

    child.terminate();
}

#[test]
fn no_restart_flag_tears_the_session_down_on_a_single_crash() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!("echo x >> \"{}\"\nexit 1", counter.display()),
    );

    let (child, _out_path) = fx.spawn_serve(&["--no-restart"]);

    let exited = wait_until(Duration::from_secs(3), || !process_is_live(child.id()));
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

    wait_until(Duration::from_secs(3), || !process_is_live(child.id()));
    child.terminate();

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

    assert!(
        process_is_live(child.id()),
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

    child.terminate();
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
    assert!(
        process_is_live(child.id()),
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

    child.terminate();
}

/// Fix round 1, Critical 1. Every earlier `--json` test in this suite
/// passes `--frontend-only`, which is exactly the flag that gates off
/// the TypeScript-regeneration file watcher (`!skip_types &&
/// !frontend_only` in `run()`) - so none of them ever touched the code
/// path `start_type_watcher` runs on. This test forces that path with
/// `--backend-only` instead (still hermetic: `Cargo.toml` + an empty
/// `src/` satisfy `validate_suprnova_project` and the watcher's
/// `watcher.watch("src", ...)`, and a shimmed `cargo` satisfies both
/// `ensure_cargo_watch`'s `cargo watch --version` probe and the actual
/// `cargo watch -x '...'` backend spawn without a real build). Before the
/// fix, `start_type_watcher` printed its "Watching for Rust file
/// changes..." notice via a raw, unconditional `println!` that never
/// looked at `--json` - this test is what catches that regressing again.
#[test]
fn json_mode_without_frontend_only_keeps_stdout_pure_ndjson_through_the_type_watcher() {
    let fx = Fixture::new();
    fx.write_backend_project("fixture-app");
    fx.shim("cargo", CARGO_WATCH_SHIM);

    let (mut child, out_path, _err_path) = fx.spawn_serve_split_full(&["--backend-only", "--json"]);

    // The watcher prints its startup notice synchronously right after
    // `watcher.watch(src_path, ...)` succeeds - no debounce or poll
    // interval gates it, so ~1s is generous.
    std::thread::sleep(Duration::from_millis(1000));

    assert!(
        process_is_live(child.id()),
        "the session must still be running"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    assert!(!output.is_empty(), "--json mode must still produce output");
    for line in output.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line).unwrap_or_else(|e| {
            panic!("line {line:?} is not valid JSON (the type watcher leaked onto stdout): {e}")
        });
    }

    child.terminate();
}

/// Fix round 1, Critical 2. The earlier behavior tests historically
/// stopped the child with `child.kill()` (SIGKILL), so the `ctrlc`
/// handler - the normal way `suprnova serve --json` actually stops - was never
/// exercised by anything here. Before the fix, that handler printed a
/// blank `println!()` and a human "Shutting down servers..." line with
/// no `--json` guard, which would land on stdout ahead of the final
/// `Shutdown` NDJSON event. Sending a real `SIGINT` is what catches that.
#[test]
fn sigint_shuts_down_with_stdout_staying_pure_ndjson_through_the_final_event() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    let descendant_pid_path = fx.root().join("npm-descendant.pid");
    fx.shim(
        "npm",
        &format!(
            "trap 'exit 0' TERM INT\n\
             echo shim-alive\n\
             echo x >> \"{}\"\n\
             sleep 10 &\n\
             descendant=$!\n\
             printf '%s\\n' \"$descendant\" > \"{}\"\n\
             wait \"$descendant\"",
            counter.display(),
            descendant_pid_path.display()
        ),
    );

    let (mut child, out_path, _err_path) = fx.spawn_serve_split(&["--json"]);
    let pgid = child.process_group_id();
    assert!(wait_until(Duration::from_secs(3), || descendant_pid_path.exists()));
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .expect("read npm descendant pid")
        .trim()
        .parse::<u32>()
        .expect("parse npm descendant pid");
    assert!(process_exists(descendant_pid));

    let pid = i32::try_from(child.id()).expect("child pid fits in pid_t");
    kill(Pid::from_raw(pid), Signal::SIGINT).expect("send SIGINT");

    let exited = wait_until(Duration::from_secs(3), || !process_is_live(child.id()));
    assert!(exited, "SIGINT must shut the session down");
    child.terminate();
    assert!(
        !process_group_exists(pgid),
        "SIGINT cleanup must not leave process-group survivors"
    );
    assert!(
        !process_exists(descendant_pid),
        "SIGINT cleanup must not leave output-retaining descendants"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    assert!(!output.is_empty(), "--json mode must still produce output");

    let mut last_type = None;
    for line in output.lines().filter(|l| !l.is_empty()) {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|e| {
            panic!("line {line:?} is not valid JSON (Ctrl+C leaked onto stdout): {e}")
        });
        last_type = value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    assert_eq!(
        last_type.as_deref(),
        Some("shutdown"),
        "the final stdout line under --json must be the Shutdown event, not a stray human line"
    );
}

/// Important 3's give-up path: `--restart-tries` bounds the crash loop
/// instead of retrying a permanently broken process forever. `1` gives
/// up after the second consecutive crash (the first schedules a 200ms
/// respawn; the respawned child crashing again exceeds the limit), which
/// keeps this test fast without needing the 5-attempt default's full
/// backoff climb.
#[test]
fn restart_tries_exhausted_gives_up_on_the_process_but_leaves_the_session_running() {
    let fx = Fixture::new();
    let counter = fx.root().join("npm-invocations");
    fx.shim(
        "npm",
        &format!("echo x >> \"{}\"\nexit 1", counter.display()),
    );

    let (mut child, out_path, _err_path) =
        fx.spawn_serve_split(&["--json", "--restart-tries", "1"]);

    // Two 100ms poll ticks plus the 200ms floor delay between the two
    // crashes that exhaust the limit; 1.5s is generous slack.
    std::thread::sleep(Duration::from_millis(1500));

    assert!(
        process_is_live(child.id()),
        "giving up on one process must not tear the whole session down \
         (matches Laravel's own concurrently --restart-tries default)"
    );

    let output = fs::read_to_string(&out_path).unwrap_or_default();
    let gave_up = output
        .lines()
        .filter(|l| !l.is_empty())
        .find_map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            (value.get("type").and_then(Value::as_str) == Some("gave_up")).then_some(value)
        })
        .unwrap_or_else(|| panic!("no gave_up event in output: {output}"));
    assert_eq!(
        gave_up.get("name").and_then(Value::as_str),
        Some("frontend")
    );
    assert_eq!(gave_up.get("tries").and_then(Value::as_u64), Some(1));

    child.terminate();
}

#[test]
fn dropping_serve_fixture_reaps_the_fake_cargo_process_group() {
    let fx = Fixture::new();
    fx.write_backend_project("fixture-app");
    fx.shim("cargo", CARGO_WATCH_DESCENDANT_SHIM);

    let (child, _, _) = fx.spawn_serve_split_full(&["--backend-only", "--json"]);
    let pgid = child.process_group_id();
    let descendant_pid_path = fx.root().join("cargo-descendant.pid");
    assert!(wait_until(Duration::from_secs(3), || descendant_pid_path.exists()));
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .expect("read cargo descendant pid")
        .trim()
        .parse::<u32>()
        .expect("parse cargo descendant pid");
    assert!(process_group_exists(pgid));
    assert!(process_exists(descendant_pid));

    drop(child);

    assert!(!process_group_exists(pgid));
    assert!(!process_exists(descendant_pid));
}

#[test]
fn terminating_serve_fixture_closes_descendant_output_descriptor() {
    let fx = Fixture::new();
    fx.write_backend_project("fixture-app");
    fx.shim("cargo", CARGO_WATCH_DESCENDANT_SHIM);

    let (mut child, mut output) = fx.spawn_serve_socket_full(&["--backend-only", "--json"]);
    let pgid = child.process_group_id();
    let descendant_pid_path = fx.root().join("cargo-descendant.pid");
    assert!(wait_until(Duration::from_secs(3), || descendant_pid_path.exists()));
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .expect("read cargo descendant pid")
        .trim()
        .parse::<u32>()
        .expect("parse cargo descendant pid");
    assert!(process_exists(descendant_pid));

    child.terminate();

    assert!(!process_group_exists(pgid));
    assert!(!process_exists(descendant_pid));
    output
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set output read timeout");
    let mut captured = String::new();
    output
        .read_to_string(&mut captured)
        .expect("all process-group output descriptors must close");
    assert!(captured.contains("backend-shim-alive"), "{captured}");
}

#[test]
fn unwinding_serve_fixture_reaps_the_group_and_closes_descendant_output() {
    let fx = Fixture::new();
    fx.write_backend_project("fixture-app");
    fx.shim("cargo", CARGO_WATCH_DESCENDANT_SHIM);
    let descendant_pid_path = fx.root().join("cargo-descendant.pid");
    let mut pgid = None;
    let mut output = None;

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (child, socket) = fx.spawn_serve_socket_full(&["--backend-only", "--json"]);
        pgid = Some(child.process_group_id());
        output = Some(socket);
        assert!(wait_until(Duration::from_secs(3), || descendant_pid_path.exists()));
        panic!("intentional fixture unwind");
    }));

    assert!(unwind.is_err(), "fixture scope must unwind");
    let pgid = pgid.expect("record process group id before unwind");
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .expect("read cargo descendant pid")
        .trim()
        .parse::<u32>()
        .expect("parse cargo descendant pid");
    assert!(!process_group_exists(pgid));
    assert!(!process_exists(descendant_pid));

    let mut output = output.expect("retain output reader across unwind");
    output
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set output read timeout");
    let mut captured = String::new();
    output
        .read_to_string(&mut captured)
        .expect("unwind must close every process-group output descriptor");
    assert!(captured.contains("backend-shim-alive"), "{captured}");
}

#[test]
fn failed_group_signaling_falls_back_to_descendant_cleanup_without_blocking() {
    let temp_dir = tempdir().expect("create descendant fixture directory");
    let descendant_pid_path = temp_dir.path().join("descendant.pid");
    let (mut output, child_output) = UnixStream::pair().expect("create descendant output socket");
    let child_output: OwnedFd = child_output.into();
    let child = Command::new("sh")
        .arg("-c")
        .arg(
            "sleep 7 &\n\
             descendant=$!\n\
             printf '%s\\n' \"$descendant\" > \"$1\"\n\
             printf 'descendant-output-open\\n'\n\
             wait \"$descendant\"",
        )
        .arg("failed-signal-leader")
        .arg(&descendant_pid_path)
        .stdout(Stdio::from(child_output))
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn process-group leader with descendant");
    let leader_pid = child.id();
    let mut child = ProcessGroupChild::new(child);
    assert!(wait_until(Duration::from_secs(2), || {
        descendant_pid_path.exists()
    }));
    let descendant_pid = fs::read_to_string(&descendant_pid_path)
        .expect("read descendant pid")
        .trim()
        .parse::<u32>()
        .expect("parse descendant pid");
    assert!(process_exists(descendant_pid));

    let mut signal_attempts = Vec::new();
    let started = Instant::now();
    child.terminate_with_signal(|pgid, signal| {
        assert_eq!(pgid, leader_pid);
        signal_attempts.push(signal);
        false
    });
    let elapsed = started.elapsed();
    let leader_gone_at_return = !process_exists(leader_pid);
    let descendant_gone_at_return = !process_exists(descendant_pid);
    let group_gone_at_return = !process_group_exists(leader_pid);

    // On a broken implementation, let the short-lived descendant exit
    // naturally before asserting so the regression cannot leak a process.
    assert!(wait_until(Duration::from_secs(3), || {
        !process_group_exists(leader_pid)
    }));
    output
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set descendant output timeout");
    let mut captured = String::new();
    output
        .read_to_string(&mut captured)
        .expect("descendant cleanup must close its output descriptor");

    assert_eq!(signal_attempts, [Signal::SIGTERM]);
    assert!(
        elapsed < Duration::from_secs(5),
        "failed group signaling must return within the cleanup bound"
    );
    assert!(child.child.is_none(), "cleanup must disarm the guard");
    assert!(leader_gone_at_return, "fallback must reap the leader");
    assert!(
        descendant_gone_at_return,
        "fallback must not leave the recorded descendant alive"
    );
    assert!(
        group_gone_at_return,
        "fallback must not leave live process-group members"
    );
    assert!(captured.contains("descendant-output-open"), "{captured}");
}

#[test]
fn natural_leader_exit_remains_unreaped_until_delayed_cleanup_disarms_guard() {
    let fx = Fixture::new();
    fx.shim("npm", "exit 1");

    let (mut child, _out_path) = fx.spawn_serve(&["--no-restart"]);
    let leader_pid = child.id();
    assert!(
        wait_until(Duration::from_secs(3), || process_is_zombie(leader_pid)),
        "naturally exited leader must remain as an unreaped zombie"
    );

    std::thread::sleep(Duration::from_millis(100));
    assert!(
        process_is_zombie(leader_pid),
        "delayed cleanup must not reap the leader while the guard can still signal its group"
    );

    child.terminate();
    assert!(
        child.child.is_none(),
        "cleanup must disarm the guard after reaping the leader"
    );
    assert!(
        !process_exists(leader_pid),
        "cleanup must reap the naturally exited leader"
    );

    child.terminate();
    assert!(
        child.child.is_none(),
        "repeated cleanup must not restore stale signaling authority"
    );
}
