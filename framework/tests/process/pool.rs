//! PAR-024: pools with bounded concurrency, and pipes.

use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::Process;

use crate::support::sh;

/// A script that counts the slots taken in `dir` as it starts, holds one
/// for a second, and gives it back.
fn slot(dir: &std::path::Path) -> [String; 3] {
    sh(&format!(
        "mkdir '{d}/slot.'$$; ls '{d}' | grep -c slot >> '{d}/../counts'; sleep 1; rmdir '{d}/slot.'$$",
        d = dir.display()
    ))
}

#[tokio::test]
#[serial]
async fn a_pool_with_concurrency_two_runs_at_most_two_at_once() {
    let root = tempfile::tempdir().unwrap();
    let slots = root.path().join("slots");
    std::fs::create_dir(&slots).unwrap();

    let mut pool = Process::pool().concurrency(2);
    for _ in 0..6 {
        pool = pool.push(Process::command(slot(&slots)));
    }
    let started = Instant::now();
    let results = pool.run().await;

    assert!(
        started.elapsed() >= Duration::from_millis(2900),
        "six one-second jobs two at a time take three seconds: {:?}",
        started.elapsed()
    );
    let counts: Vec<u32> = std::fs::read_to_string(root.path().join("counts"))
        .unwrap()
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect();
    assert_eq!(counts.len(), 6);
    assert!(counts.iter().all(|count| *count <= 2), "{counts:?}");
    assert!(results.successful());
}

#[tokio::test]
#[serial]
async fn a_pool_without_a_bound_runs_everything_at_once() {
    let started = Instant::now();
    let results = Process::pool()
        .push(Process::command(["sleep", "1"]))
        .push(Process::command(["sleep", "1"]))
        .push(Process::command(["sleep", "1"]))
        .run()
        .await;
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert_eq!(results.len(), 3);
}

#[tokio::test]
#[serial]
async fn pool_results_are_keyed_in_the_order_added_and_a_failure_stops_nothing() {
    let results = Process::pool()
        .add("slow", Process::command(sh("sleep 0.3; printf slow")))
        .add("fails", Process::command(sh("printf broke >&2; exit 2")))
        .push(Process::command(["printf", "third"]))
        .run()
        .await;

    let keys: Vec<&str> = results.keys().collect();
    assert_eq!(
        keys,
        ["slow", "fails", "2"],
        "named keys, then the position"
    );
    assert_eq!(
        results.get("slow").unwrap().as_ref().unwrap().output(),
        "slow"
    );
    let failed = results.get("fails").unwrap().as_ref().unwrap();
    assert_eq!(failed.exit_code(), Some(2));
    assert_eq!(
        results.get("2").unwrap().as_ref().unwrap().output(),
        "third"
    );
    assert!(!results.successful());
    assert_eq!(results.failed().collect::<Vec<_>>(), ["fails"]);
}

#[tokio::test]
#[serial]
async fn a_started_pool_runs_its_processes_and_waits_for_them() {
    let mut pool = Process::pool()
        .add("a", Process::command(sh("sleep 0.3; printf a")))
        .add("b", Process::command(sh("sleep 0.3; printf b")))
        .start();
    assert!(pool.running());
    let results = pool.wait().await;
    assert_eq!(results.get("a").unwrap().as_ref().unwrap().output(), "a");
    assert_eq!(results.get("b").unwrap().as_ref().unwrap().output(), "b");
}

#[tokio::test]
#[serial]
async fn a_pipe_feeds_each_output_to_the_next_input() {
    let result = Process::pipe()
        .push(Process::command(["printf", "b\\na\\n"]))
        .push(Process::command(["sort"]))
        .run()
        .await
        .unwrap();
    assert_eq!(result.output(), "a\nb\n");
    assert!(result.successful());
}

#[tokio::test]
#[serial]
async fn a_pipe_stops_at_the_first_failure() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let result = Process::pipe()
        .add("first", Process::command(sh("printf nope >&2; exit 5")))
        .add(
            "second",
            Process::command(["touch", marker.to_str().unwrap()]),
        )
        .run()
        .await
        .unwrap();

    assert_eq!(result.exit_code(), Some(5), "the first failed result");
    assert_eq!(result.error_output(), "nope");
    assert!(!marker.exists(), "the second process did not run");
}
