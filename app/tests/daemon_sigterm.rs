//! The long-running daemons must drain on SIGTERM, not only on Ctrl-C.
//!
//! This lives in the dogfood app rather than `framework/tests/` because
//! the thing under test is a *process*: the framework is a library and has
//! no binary to signal. `CARGO_BIN_EXE_app` gives us the real one.
//!
//! # What this caught
//!
//! `schedule:work`, `queue:work` and `workflow:work` each selected on
//! `tokio::signal::ctrl_c()` alone. `ctrl_c` installs a SIGINT handler, so
//! SIGTERM had no handler anywhere in the process — and SIGTERM is what
//! `docker stop`, Coolify, systemd and Kubernetes send. Every one of the
//! three had a careful bounded drain sitting behind that `select!` which
//! had never run under any supervisor.
//!
//! Measured on the benchmark host before the fix: `docker stop` on a
//! `queue:work` container burned its entire 40s grace window and exited
//! 137 (SIGKILL) with the in-flight job destroyed. The same worker at a
//! normal pid exited 143 in 0.1s — the default disposition, which is what
//! proves no handler existed. As PID 1, which is what a container runs,
//! the kernel discards an unhandled SIGTERM outright.
//!
//! # What this cannot cover
//!
//! The PID-1 half. A test process is never PID 1, so the "signal is
//! discarded entirely" behaviour needs a container to observe and cannot
//! be reached from here. What this file asserts is the thing that
//! *causes* it — whether a handler exists at all — which is also the
//! thing a future refactor could silently drop.

#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const APP_BIN: &str = env!("CARGO_BIN_EXE_app");

/// How long to wait for the daemon to finish booting and print its banner.
/// Generous because bootstrap opens a database and registers every
/// inventory item; a slow CI box should not fail this as though it were a
/// signal-handling defect.
const BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a signalled daemon gets to exit. The drains themselves are
/// bounded well below this (5s for the queue worker, 30s for the
/// scheduler's background tasks), so anything approaching it means the
/// signal was never observed.
const EXIT_TIMEOUT: Duration = Duration::from_secs(45);

/// Spawn a daemon subcommand with just enough environment to boot.
///
/// `QUEUE_DRIVER` is left unset on purpose: the default in-memory driver
/// needs no `jobs` table, and this file is about signal handling, not
/// about queue storage. `APP_ENV` stays out of production so the Inertia
/// manifest and rate-limiter guards do not demand a built frontend.
fn daemon_command(subcommand: &str, db_path: &Path) -> Command {
    let mut cmd = Command::new(APP_BIN);
    cmd.arg(subcommand)
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", db_path.display()),
        )
        .env("APP_ENV", "testing")
        .env("APP_DEBUG", "false")
        .env("LOG_LEVEL", "warn")
        // A `.env` in the app directory would otherwise override every
        // variable set here, pointing the test at the developer's own
        // database.
        .current_dir(db_path.parent().expect("temp db has a parent"));
    cmd
}

/// Migrate the throwaway database before a daemon touches it.
///
/// `bootstrap::register` wires the feature-flag chain against the
/// `features` table and panics if it is missing, so an unmigrated database
/// takes the daemon down during boot — which would look exactly like a
/// signal-handling failure from the outside.
fn migrate(db_path: &Path) {
    let output = daemon_command("migrate", db_path)
        .output()
        .unwrap_or_else(|e| panic!("spawn `{APP_BIN} migrate`: {e}"));
    assert!(
        output.status.success(),
        "migrating the test database failed ({:?}):\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn spawn_daemon(subcommand: &str, db_path: &Path) -> Child {
    daemon_command(subcommand, db_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn `{APP_BIN} {subcommand}`: {e}"))
}

/// Block until the daemon prints a line containing `needle`.
///
/// Reading the banner rather than sleeping a fixed interval: signalling a
/// process that has not finished booting proves nothing, and a fixed sleep
/// is either too short on a loaded machine or wasted everywhere else.
fn wait_for_line(child: &mut Child, needle: &str) -> Result<(), String> {
    let stdout = child.stdout.take().expect("stdout was piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    let deadline = Instant::now() + BOOT_TIMEOUT;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                let matched = line.contains(needle);
                seen.push(line);
                if matched {
                    return Ok(());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!(
                        "daemon exited during boot with {status}; stdout so far:\n{}",
                        seen.join("\n")
                    ));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Err(format!(
        "daemon never printed a line containing {needle:?} within {}s; stdout:\n{}",
        BOOT_TIMEOUT.as_secs(),
        seen.join("\n")
    ))
}

fn signal(child: &Child, name: &str) {
    let status = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(child.id().to_string())
        .status()
        .unwrap_or_else(|e| panic!("send {name}: {e}"));
    assert!(status.success(), "`kill -{name}` failed: {status}");
}

/// Wait for exit, returning how long it took. `None` means it outlived the
/// deadline and was killed.
fn wait_for_exit(child: &mut Child) -> Option<(std::process::ExitStatus, Duration)> {
    let started = Instant::now();
    let deadline = started + EXIT_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait().expect("try_wait") {
            Some(status) => return Some((status, started.elapsed())),
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Drive one daemon through one signal and assert it drained cleanly.
///
/// The assertion is on a *clean* exit, not merely a prompt one. Exit 143
/// is 128+15: the process died on SIGTERM's default disposition, which
/// means no handler ran and therefore no drain ran either — prompt, and
/// exactly the bug.
fn assert_daemon_drains(subcommand: &str, banner: &str, sig: &str) {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let db = tmp.path().join("daemon-sigterm.db");
    migrate(&db);

    let mut child = spawn_daemon(subcommand, &db);
    if let Err(why) = wait_for_line(&mut child, banner) {
        let stderr = child
            .stderr
            .take()
            .map(|mut e| {
                use std::io::Read;
                let mut buf = String::new();
                let _ = e.read_to_string(&mut buf);
                buf
            })
            .unwrap_or_default();
        let _ = child.kill();
        let _ = child.wait();
        panic!("`{subcommand}` did not reach its banner: {why}\nstderr:\n{stderr}");
    }

    signal(&child, sig);

    let Some((status, took)) = wait_for_exit(&mut child) else {
        panic!(
            "`{subcommand}` was still running {}s after {sig} and had to be killed. \
             Nothing is listening for the signal.",
            EXIT_TIMEOUT.as_secs()
        );
    };

    assert_eq!(
        status.code(),
        Some(0),
        "`{subcommand}` exited {:?} after {sig} in {took:?}; a clean drain exits 0. \
         143 means the default disposition killed it, so no handler ran and the \
         drain behind the select! never executed.",
        status.code()
    );
}

#[test]
fn queue_worker_drains_on_sigterm() {
    assert_daemon_drains("queue:work", "Stop with Ctrl+C or SIGTERM", "TERM");
}

#[test]
fn scheduler_drains_on_sigterm() {
    assert_daemon_drains("schedule:work", "Stop with Ctrl+C or SIGTERM", "TERM");
}

/// The workflow worker gets a weaker assertion than its siblings, and the
/// reason is the environment rather than the code: `workflow:work`
/// requires Postgres (`framework/src/workflow`), and this file runs on a
/// throwaway SQLite file. Against SQLite the worker sits in a claim-error
/// backoff loop, so it cannot exit 0 no matter how well it handles the
/// signal.
///
/// What is still worth asserting is the regression this file exists for:
/// that a handler *exists*. A process killed by a signal reports no exit
/// code at all — `status.code()` is `None` — so a `Some(_)` is proof the
/// process chose its own exit rather than being terminated by the
/// disposition. Demanding exit 0 here would only be testing that Postgres
/// is absent.
#[test]
fn workflow_worker_handles_sigterm_rather_than_dying_from_it() {
    let tmp = tempfile::TempDir::new().expect("tmpdir");
    let db = tmp.path().join("daemon-sigterm.db");
    migrate(&db);

    let mut child = spawn_daemon("workflow:work", &db);
    if let Err(why) = wait_for_line(&mut child, "Stop with Ctrl+C or SIGTERM") {
        let _ = child.kill();
        let _ = child.wait();
        panic!("`workflow:work` did not reach its banner: {why}");
    }

    signal(&child, "TERM");

    let Some((status, took)) = wait_for_exit(&mut child) else {
        panic!(
            "`workflow:work` was still running {}s after SIGTERM and had to be killed. \
             Nothing is listening for the signal.",
            EXIT_TIMEOUT.as_secs()
        );
    };

    assert!(
        status.code().is_some(),
        "`workflow:work` was terminated by a signal after {took:?} instead of exiting \
         on its own. A `None` exit code means the default disposition killed it, so no \
         handler was installed and the drain never ran."
    );
}

/// The interactive path has to keep working. Adding SIGTERM by *replacing*
/// the Ctrl-C arm would satisfy every test above and break the one thing
/// developers do every day.
#[test]
fn queue_worker_still_drains_on_sigint() {
    assert_daemon_drains("queue:work", "Stop with Ctrl+C or SIGTERM", "INT");
}
