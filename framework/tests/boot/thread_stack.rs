//! The runtime `#[suprnova::main]` builds gives its threads an 8 MiB stack.
//!
//! Tokio's default is 2 MiB per worker and blocking thread. In a debug
//! build the frames the framework puts under a Live action (the server, the
//! middleware, the session, the render cache and the Live engine) use about
//! 1.75 MiB of that, so an action a little deeper overflowed the stack and
//! aborted the server. The child process below runs a frame of 3 MiB on a
//! worker thread and on a blocking thread: it overflows the default and
//! fits the size the macro sets.

use std::process::Command;

const CHILD_MODE: &str = "SUPRNOVA_THREAD_STACK_CHILD";

/// Larger than Tokio's default stack, smaller than the macro's.
const DEEP_FRAME: usize = 3 * 1024 * 1024;

/// Holds `DEEP_FRAME` bytes in one stack frame. `black_box` keeps the
/// array, so the frame is as large in a release build as in a debug one.
fn deep_frame() -> usize {
    let frame = [0u8; DEEP_FRAME];
    std::hint::black_box(&frame).len()
}

#[suprnova::main(flavor = "multi_thread", worker_threads = 2)]
async fn a_deep_frame_on_each_kind_of_thread() -> Result<(), String> {
    let on_worker = tokio::spawn(async { deep_frame() })
        .await
        .map_err(|e| format!("the worker task failed: {e}"))?;
    let on_blocking = tokio::task::spawn_blocking(deep_frame)
        .await
        .map_err(|e| format!("the blocking task failed: {e}"))?;
    if on_worker == DEEP_FRAME && on_blocking == DEEP_FRAME {
        Ok(())
    } else {
        Err(format!(
            "the frames were {on_worker} and {on_blocking} bytes, not {DEEP_FRAME}"
        ))
    }
}

#[test]
fn thread_stack_child() {
    if std::env::var(CHILD_MODE).is_err() {
        return;
    }
    a_deep_frame_on_each_kind_of_thread().expect("a 3 MiB frame must fit a runtime thread");
}

#[test]
fn runtime_threads_hold_a_frame_larger_than_tokios_default() {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", "thread_stack::thread_stack_child", "--nocapture"])
        .env(CHILD_MODE, "1")
        .output()
        .expect("spawn the child");

    assert!(
        output.status.success(),
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
        "child filter matched no test; stdout:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}
