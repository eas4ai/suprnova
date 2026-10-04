//! Run one test of a binary as the only test of a child process.
//!
//! Under plain `cargo test` every test of a binary is a thread of one
//! process. A test that needs some process-wide state to itself cannot
//! share the process with the others: a slot of the process container
//! that the first writer keeps, a key ring sealed by the first installer,
//! a one-shot hook that the next caller on any thread consumes, or a
//! decision that the first write fixes for the life of the process. Each
//! test that uses this module says which state it needs.
//!
//! Such a test is split in two. The `_child` test holds the body and
//! returns at once unless [`is_child`] is true. The test of the original
//! name calls [`run_alone`], which starts the binary again with only the
//! `_child` test selected and fails unless it ran and passed. Under
//! nextest every test already has its own process; the child costs one
//! more start of the binary.
//!
//! Shared across test binaries via `#[path]`. A binary that declares it
//! also declares `env_lock`, which [`run_alone`] takes.

use std::process::{Command, Stdio};

/// Turns the `_child` tests from no-ops into their real body. Only
/// [`run_alone`] sets it, in the environment of the process it starts.
const OWN_PROCESS: &str = "SUPRNOVA_TEST_OWN_PROCESS";

/// Whether this process is a child that [`run_alone`] started.
pub fn is_child() -> bool {
    std::env::var_os(OWN_PROCESS).is_some()
}

/// Run the test `name` (its full path in the binary) as the only test of
/// a child process, and fail unless it ran and passed.
///
/// Sync tests only: it takes the environment lock through its blocking
/// entry point. An async test uses `own_process_async::delegate`.
pub fn run_alone(name: &str) {
    let child = {
        // The child inherits the environment. Another test can be between
        // setting a variable and restoring it; the lock waits that out.
        let _env = crate::env_lock::lock_env();
        child_command(name)
            .spawn()
            .expect("spawn the child process")
    };
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    assert_child_passed(&output);
}

/// The command that runs the test `name` alone in a child of this test
/// binary, with stdout and stderr captured for [`assert_child_passed`].
pub fn child_command(name: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", name, "--nocapture"])
        .env(OWN_PROCESS, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Fail unless the child ran exactly one test and passed it.
pub fn assert_child_passed(output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("running 1 test"),
        "the child filter matched no test (the module path changed?); stdout:\n{stdout}"
    );
    assert!(
        output.status.success(),
        "the child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
