//! Helpers shared by the concurrency tests.

use std::path::Path;
use std::time::{Duration, Instant};
use suprnova::Concurrency;

/// This test binary's path.
pub fn this_binary() -> String {
    std::env::current_exe()
        .expect("the test binary has a path")
        .to_str()
        .expect("the test binary's path is UTF-8")
        .to_owned()
}

/// The command that starts this binary as the console, entering through
/// `child::entry`, after `prefix` (such as `env NAME=value`).
pub fn console_command(prefix: &[&str]) -> Vec<String> {
    prefix
        .iter()
        .map(ToString::to_string)
        .chain([
            this_binary(),
            "child::entry".to_owned(),
            "--exact".to_owned(),
            "--nocapture".to_owned(),
        ])
        .collect()
}

/// Point the process driver at this binary.
pub fn use_this_binary() {
    Concurrency::console_command(console_command(&[])).expect("the console command is set");
}

/// Wait up to `within` for `check` to hold.
pub async fn wait_until(within: Duration, mut check: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if check() {
            return true;
        }
        if started.elapsed() > within {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether `file` exists.
pub fn exists(file: &Path) -> bool {
    file.exists()
}
