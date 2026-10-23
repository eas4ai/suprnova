//! Run an async test alone in a child process, from inside the test.
//!
//! The async sibling of `own_process::run_alone`, for a test binary whose
//! tests install process-wide state, such as the Magnetar engines, that two
//! tests cannot share. A test that needs that state to itself starts with
//!
//! ```ignore
//! if crate::own_process_async::delegate(module_path!(), "its_own_name").await {
//!     return;
//! }
//! ```
//!
//! Under plain `cargo test` the call runs the same test again as the only
//! test of a child process and returns `true` once the child passed, so the
//! parent returns at once. In that child it returns `false`, and the body
//! runs with the process to itself. Under nextest every test already has its
//! own process; the child costs one more start of the binary.
//!
//! Shared via `#[path]`. A binary that declares it also declares
//! `own_process` and `env_lock`.

/// Run the calling test alone in a child process unless this process is
/// that child. `module` is the caller's `module_path!()` and `test` its
/// function name; together they name the test as the binary lists it.
pub async fn delegate(module: &str, test: &str) -> bool {
    if crate::own_process::is_child() {
        return false;
    }
    // A test's path inside its binary leaves out the crate name.
    let module = module.split_once("::").map_or("", |(_, rest)| rest);
    let name = if module.is_empty() {
        test.to_owned()
    } else {
        format!("{module}::{test}")
    };
    let child = {
        // The child inherits the environment. Another test can be between
        // setting a variable and restoring it; the lock waits that out
        // without blocking the runtime.
        let _env = crate::env_lock::lock_env_async().await;
        crate::own_process::child_command(&name)
            .spawn()
            .expect("spawn the child process")
    };
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .expect("the wait for the child process did not panic")
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
    true
}
