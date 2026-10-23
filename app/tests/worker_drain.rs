//! MEM-006: a worker waits for the queued listeners its jobs started
//! before the process exits, as the server does.
//!
//! The thing under test is a process exit, so this drives the real `app`
//! and `console` binaries over a throwaway SQLite database. The
//! `bench:enqueue-listener` command pushes a `BenchListener` job onto the
//! database queue; its handler fires a queued event whose listener writes
//! `<path>.started`, sleeps, and writes `path` last. Before the fix the
//! worker returned while that listener slept and the runtime dropped it
//! with the process, so `path` never appeared.
//!
//! `schedule:run` is driven through the app's `bench:queued-listener`
//! task, which fires the same event when `BENCH_LISTENER_PATH` names the
//! file. `schedule:work` and the workflow worker end through the same
//! shutdown step as `queue:work`. Neither can be driven here: the daemon
//! scheduler ticks on minute boundaries, and `workflow:work` needs
//! Postgres (see `daemon_sigterm.rs`).

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const APP_BIN: &str = env!("CARGO_BIN_EXE_app");
const CONSOLE_BIN: &str = env!("CARGO_BIN_EXE_console");

/// Long enough that a worker which does not wait exits well before the
/// listener writes, short enough to keep the test quick.
const LISTENER_MILLIS: u64 = 1_500;

const EXIT_TIMEOUT: Duration = Duration::from_secs(45);

fn command(bin: &str, db: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env(
        "DATABASE_URL",
        format!("sqlite://{}?mode=rwc", db.display()),
    )
    .env("APP_ENV", "testing")
    .env("APP_DEBUG", "false")
    .env("LOG_LEVEL", "warn")
    .env("QUEUE_DRIVER", "database")
    // A `.env` in the app directory would otherwise override every
    // variable set here.
    .current_dir(db.parent().expect("temp db has a parent"));
    cmd
}

fn run(bin: &str, db: &Path, args: &[&str]) -> Output {
    let output = command(bin, db)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawn `{bin} {args:?}`: {e}"));
    assert!(
        output.status.success(),
        "`{bin} {args:?}` failed ({:?}):\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

/// A migrated database with one `BenchListener` job waiting on it.
fn enqueued(tmp: &Path) -> (PathBuf, PathBuf) {
    let db = tmp.join("worker-drain.db");
    let done = tmp.join("listener.done");
    run(APP_BIN, &db, &["migrate"]);
    run(
        CONSOLE_BIN,
        &db,
        &[
            "bench:enqueue-listener",
            "--path",
            done.to_str().expect("utf-8 temp path"),
            "--millis",
            &LISTENER_MILLIS.to_string(),
        ],
    );
    (db, done)
}

fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!(
        "the worker was still running {}s on",
        EXIT_TIMEOUT.as_secs()
    );
}

#[test]
fn mem_audit_a_worker_that_reaches_max_jobs_waits_for_its_listeners() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let (db, done) = enqueued(tmp.path());

    let output = run(APP_BIN, &db, &["queue:work", "--max-jobs", "1"]);

    assert!(
        done.with_extension("done.started").exists(),
        "the listener never started, so this run proves nothing:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        done.exists(),
        "the worker exited while its queued listener ran"
    );
}

#[test]
fn mem_audit_a_signalled_worker_waits_for_its_listeners() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let (db, done) = enqueued(tmp.path());
    let started = done.with_extension("done.started");

    let mut child = command(APP_BIN, &db)
        .arg("queue:work")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn `{APP_BIN} queue:work`: {e}"));

    let deadline = Instant::now() + EXIT_TIMEOUT;
    while !started.exists() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the listener never started");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let status = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("send TERM");
    assert!(status.success(), "`kill -TERM` failed: {status}");

    let status = wait_for_exit(&mut child);
    assert_eq!(status.code(), Some(0), "the worker did not drain cleanly");
    assert!(
        done.exists(),
        "the worker exited while its queued listener ran"
    );
}

#[test]
fn mem_audit_schedule_run_waits_for_its_listeners() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let db = tmp.path().join("schedule-drain.db");
    let done = tmp.path().join("listener.done");
    run(APP_BIN, &db, &["migrate"]);
    let output = command(APP_BIN, &db)
        .arg("schedule:run")
        .env("BENCH_LISTENER_PATH", &done)
        .output()
        .unwrap_or_else(|e| panic!("spawn `{APP_BIN} schedule:run`: {e}"));
    assert!(
        done.with_extension("done.started").exists(),
        "the listener never started, so this run proves nothing:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        done.exists(),
        "schedule:run exited while its queued listener ran"
    );
}
