//! Helpers for watching real processes from the outside.

use std::path::Path;
use std::time::{Duration, Instant};

/// A shell command line for `sh -c`, as the arguments of `Process::command`.
pub fn sh(script: &str) -> [String; 3] {
    ["sh".into(), "-c".into(), script.into()]
}

/// Whether a process with this id is alive, by `kill -0`.
pub fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Wait up to `within` for every process in `pids` to be gone.
pub async fn all_gone(pids: &[u32], within: Duration) -> bool {
    let start = Instant::now();
    loop {
        if pids.iter().all(|pid| !alive(*pid)) {
            return true;
        }
        if start.elapsed() > within {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Wait up to five seconds for `file` to hold `count` lines, each a process
/// id, and return them.
pub async fn pids_in(file: &Path, count: usize) -> Vec<u32> {
    let start = Instant::now();
    loop {
        let pids: Vec<u32> = std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        if pids.len() >= count {
            return pids;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the process never wrote {count} ids to {}",
            file.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A script that writes its own id and its background child's id to
/// `file`, then waits on the child, so both can be watched.
pub fn parent_and_child(file: &Path) -> String {
    format!(
        "echo $$ > '{f}'; sleep 30 & echo $! >> '{f}'; wait",
        f = file.display()
    )
}
