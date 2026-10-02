//! PAR-022 and PAR-023: timeouts, idle timeouts, started processes, and
//! the cleanup of every process a command started.

use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::{Process, ProcessError, Signal};

use crate::support::{alive, all_gone, parent_and_child, pids_in, sh};

#[tokio::test]
#[serial]
async fn a_timeout_kills_the_process_and_every_process_it_started() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let started = Instant::now();

    let error = Process::command(sh(&parent_and_child(&file)))
        .timeout(Duration::from_secs(1))
        .run()
        .await
        .expect_err("thirty seconds is past a one-second timeout");

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("sh -c"), "names the command: {text}");
    assert!(text.contains("1 second"), "names the timeout: {text}");
    let pids = pids_in(&file, 2).await;
    assert!(
        all_gone(&pids, Duration::from_secs(3)).await,
        "the shell and its sleep are gone: {pids:?}"
    );
}

#[tokio::test]
#[serial]
async fn forever_removes_the_timeout() {
    let result = Process::command(["sleep", "0.3"])
        .timeout(Duration::from_millis(100))
        .forever()
        .run()
        .await
        .expect("no timeout after forever");
    assert!(result.successful());
}

#[tokio::test]
#[serial]
async fn a_started_process_reports_its_id_output_and_exit() {
    let mut process = Process::command(sh("printf first; sleep 0.3; printf second"))
        .start()
        .expect("started");
    let pid = process.id().expect("a running process has an id");
    assert!(alive(pid));
    assert!(process.running());

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(process.latest_output(), "first");
    assert_eq!(
        process.latest_output(),
        "",
        "nothing new since the last read"
    );

    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "firstsecond");
    assert!(result.successful());
}

#[tokio::test]
#[serial]
async fn running_turns_false_once_the_process_exits() {
    let mut process = Process::command(["true"]).start().unwrap();
    let start = Instant::now();
    while process.running() {
        assert!(start.elapsed() < Duration::from_secs(5), "still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(process.wait().await.unwrap().successful());
}

#[tokio::test]
#[serial]
async fn a_signal_reaches_a_started_process() {
    let process = Process::command(sh(
        "trap 'printf got-term; exit 7' TERM; while :; do sleep 0.1; done",
    ))
    .start()
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    process.signal(Signal::Term).expect("signalled");

    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "got-term");
    assert_eq!(result.exit_code(), Some(7));
}

#[tokio::test]
#[serial]
async fn stop_kills_a_process_that_ignores_the_terminate_signal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let process = Process::command(sh(&format!("trap '' TERM; {}", parent_and_child(&file))))
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;

    let started = Instant::now();
    let result = process
        .stop(Duration::from_millis(300))
        .await
        .expect("stopped");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(result.failed(), "a killed process did not succeed");
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn dropping_a_started_process_kills_it_and_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let process = Process::command(sh(&parent_and_child(&file)))
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;

    drop(process);
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn dropping_the_run_future_kills_the_process_and_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let run = Process::command(sh(&parent_and_child(&file)))
        .forever()
        .run();

    let outcome = tokio::time::timeout(Duration::from_millis(500), run).await;
    assert!(outcome.is_err(), "the run was cancelled");
    let pids = pids_in(&file, 2).await;
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn an_idle_timeout_kills_a_silent_process() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let started = Instant::now();

    let error = Process::command(sh(&parent_and_child(&file)))
        .idle_timeout(Duration::from_secs(1))
        .run()
        .await
        .expect_err("a silent process goes idle");

    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        matches!(error, ProcessError::IdleTimedOut { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("sh -c"), "{error}");
    let pids = pids_in(&file, 2).await;
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn a_process_that_keeps_writing_runs_past_the_idle_timeout() {
    let result = Process::command(sh(
        "i=0; while [ $i -lt 15 ]; do printf .; sleep 0.2; i=$((i+1)); done",
    ))
    .idle_timeout(Duration::from_secs(1))
    .run()
    .await
    .expect("steady output keeps it alive");
    assert_eq!(result.output(), ".".repeat(15));
}
