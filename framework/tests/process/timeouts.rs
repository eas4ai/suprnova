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
        .stop(Duration::from_millis(300), None)
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

// Review fixes.

#[tokio::test]
#[serial]
async fn a_tty_process_is_killed_with_everything_it_started() {
    if !Process::supports_tty() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let error = Process::command(sh(&parent_and_child(&file)))
        .tty()
        .timeout(Duration::from_secs(1))
        .run()
        .await
        .expect_err("a tty process has a timeout too");
    assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
    let pids = pids_in(&file, 2).await;
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn dropping_a_started_tty_process_kills_its_children() {
    if !Process::supports_tty() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let process = Process::command(sh(&parent_and_child(&file)))
        .tty()
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;
    drop(process);
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn an_idle_timeout_on_a_tty_process_is_refused() {
    let error = Process::command(["true"])
        .tty()
        .idle_timeout(Duration::from_secs(1))
        .run()
        .await
        .expect_err("nothing can watch a terminal's output");
    assert!(
        matches!(error, ProcessError::Unsupported { .. }),
        "{error:?}"
    );
    assert!(error.to_string().contains("idle timeout"), "{error}");
}

#[tokio::test]
#[serial]
async fn stop_kills_a_child_that_outlives_its_parent_and_ignores_the_terminate_signal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let process = Process::command(sh(&format!(
        "echo $$ > '{f}'; (trap '' TERM; exec sleep 30) & echo $! >> '{f}'; wait",
        f = file.display()
    )))
    .start()
    .unwrap();
    let pids = pids_in(&file, 2).await;

    let started = Instant::now();
    process
        .stop(Duration::from_millis(300), None)
        .await
        .expect("stopped");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "stop did not wait on the child: {:?}",
        started.elapsed()
    );
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
}

#[tokio::test]
#[serial]
async fn a_started_process_is_killed_at_its_timeout_though_nothing_waits() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let mut process = Process::command(sh(&parent_and_child(&file)))
        .timeout(Duration::from_secs(1))
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;

    let started = Instant::now();
    while process.running() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "still running past its timeout"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(all_gone(&pids, Duration::from_secs(3)).await, "{pids:?}");
    let error = process.wait().await.expect_err("the timeout passed");
    assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
}

/// The program a timeout killed is reaped though nothing looks at it, not
/// `running` and not `wait`. Unreaped, it stays a zombie, which keeps its
/// id and still answers `kill -0`.
#[tokio::test]
#[serial]
async fn a_timed_out_process_is_reaped_though_nothing_looks_at_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let mut process = Process::command(sh(&format!("printf before; {}", parent_and_child(&file))))
        .timeout(Duration::from_secs(1))
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;

    assert!(
        all_gone(&pids, Duration::from_secs(4)).await,
        "the shell or its sleep is still there: {pids:?}"
    );
    assert!(!process.running());
    let refused = process
        .signal(Signal::Term)
        .expect_err("a reaped program is not signalled");
    assert!(
        matches!(refused, ProcessError::Signal { .. }),
        "{refused:?}"
    );
    let error = process.wait().await.expect_err("the timeout passed");
    let ProcessError::TimedOut {
        timeout, result, ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(*timeout, Duration::from_secs(1));
    assert_eq!(
        result.output(),
        "before",
        "the output before the kill is kept"
    );
    assert!(error.to_string().contains("sh -c"), "{error}");
}

#[tokio::test]
#[serial]
async fn wait_until_survives_bytes_that_are_not_utf8() {
    let mut process = Process::command(sh(
        "printf '\\377'; sleep 0.2; printf '\\303'; sleep 0.2; printf '\\251 ready'",
    ))
    .start()
    .unwrap();
    let matched = process
        .wait_until(|_, chunk| chunk.contains("é ready"))
        .await
        .expect("no panic, no skipped output");
    assert!(matched);
    assert!(process.wait().await.unwrap().successful());
}

#[tokio::test]
#[serial]
async fn a_zero_timeout_is_no_timeout_and_a_huge_one_does_not_panic() {
    let zero = Process::command(["sleep", "0.3"])
        .timeout(Duration::ZERO)
        .run()
        .await
        .expect("zero means no timeout, as in Laravel");
    assert!(zero.successful());
    let huge = Process::command(["true"])
        .timeout(Duration::MAX)
        .idle_timeout(Duration::MAX)
        .run()
        .await
        .expect("a timeout past what the clock holds is none");
    assert!(huge.successful());
}

/// DRIVERS-049: a program was reaped while a process that left its group
/// still held its output, and the group was signalled afterwards on the
/// strength of the open pipe. Reaping frees the program's id, and with it
/// the group id, which an unrelated group may then take. The program has to
/// stay unreaped, its id pinned, for as long as its group may be signalled.
#[cfg(target_os = "linux")]
#[tokio::test]
#[serial]
async fn the_program_stays_unreaped_while_an_escaped_process_holds_its_output() {
    let mut process = Process::command(sh("setsid sleep 3 & exit 0"))
        .start()
        .expect("sh starts");
    let pid = process.id().expect("a real process has an id");

    tokio::time::timeout(Duration::from_secs(5), async {
        while process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the shell exits at once");

    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
    let state = stat
        .as_deref()
        .ok()
        .and_then(|stat| stat.rsplit_once(") "))
        .and_then(|(_, rest)| rest.chars().next());
    assert_eq!(
        state,
        Some('Z'),
        "the shell was reaped while the escaped sleep still held its output, so \
         its id and its group id were free for reuse"
    );
    drop(process);
}

#[tokio::test]
#[serial]
async fn stop_uses_the_requested_signal_and_can_end_before_grace() {
    let mut process = Process::shell(
        "trap 'printf interrupted; exit 7' INT; printf ready; while :; do sleep 0.1; done",
    )
    .start()
    .unwrap();
    assert!(
        process
            .wait_until(|_, chunk| chunk.contains("ready"))
            .await
            .unwrap()
    );
    let started = Instant::now();
    let result = process
        .stop(Duration::from_secs(1), Signal::Interrupt)
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(result.exit_code(), Some(7));
    assert!(result.output().contains("interrupted"));
}

#[tokio::test]
#[serial]
async fn default_stop_waits_ten_seconds_before_killing_a_term_ignoring_tree() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let process = Process::command(sh(&format!("trap '' TERM; {}", parent_and_child(&file))))
        .forever()
        .start()
        .unwrap();
    let pids = pids_in(&file, 2).await;
    let started = Instant::now();
    let result = process.stop(None, None).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert!(started.elapsed() < Duration::from_secs(14));
    assert!(result.failed());
    assert!(all_gone(&pids, Duration::from_secs(3)).await);
}

#[tokio::test]
#[serial]
async fn zero_stop_grace_kills_immediately() {
    let process = Process::command(["sleep", "30"]).start().unwrap();
    let pid = process.id().unwrap();
    let started = Instant::now();
    assert!(process.stop(Duration::ZERO, None).await.unwrap().failed());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(all_gone(&[pid], Duration::from_secs(3)).await);
}

/// Keep failed cleanup regressions from leaving their sleeps running.
#[cfg(target_os = "linux")]
struct EscapedSleeps(Vec<u32>);

#[cfg(target_os = "linux")]
impl Drop for EscapedSleeps {
    fn drop(&mut self) {
        for pid in &self.0 {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(*pid).unwrap());
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
async fn escaped_sleeps(process: &suprnova::InvokedProcess) -> EscapedSleeps {
    let root = process.id().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let children = std::fs::read_to_string(format!("/proc/{root}/task/{root}/children"))
                .unwrap_or_default();
            let pids: Vec<u32> = children
                .split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect();
            let ready: Vec<_> = pids
                .iter()
                .filter_map(|pid| {
                    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                    let (_, fields) = stat.rsplit_once(") ")?;
                    let session = fields.split_whitespace().nth(3)?.parse::<u32>().ok()?;
                    Some((*pid, stat.contains("(sleep)"), session))
                })
                .collect();
            if ready.len() == 2
                && ready.iter().all(|(_, sleep, _)| *sleep)
                && ready.iter().any(|(pid, _, session)| pid == session)
            {
                let mut all = vec![root];
                all.extend(pids);
                return EscapedSleeps(all);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both sleeps start and one leaves the process group")
}

#[cfg(target_os = "linux")]
async fn escaped_sleeps_stopped(pids: &[u32]) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let running = pids.iter().any(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        stat.rsplit_once(") ")
                            .map(|(_, fields)| !fields.starts_with('Z'))
                    })
                    .unwrap_or(false)
            });
            if !running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[serial]
async fn a_timeout_kills_descendants_that_leave_the_process_group() {
    let started = Instant::now();
    let process = Process::shell("setsid sleep 30 & sleep 30")
        .timeout(Duration::from_secs(1))
        .start()
        .unwrap();
    let sleeps = escaped_sleeps(&process).await;
    let error = process.wait().await.expect_err("one-second timeout");
    assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
    assert!(error.to_string().contains("setsid sleep 30"));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        escaped_sleeps_stopped(&sleeps.0).await,
        "escaped sleeps still running: {:?}",
        sleeps.0
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[serial]
async fn dropping_a_process_kills_descendants_that_leave_the_process_group() {
    let process = Process::shell("setsid sleep 30 & sleep 30")
        .start()
        .unwrap();
    let sleeps = escaped_sleeps(&process).await;
    drop(process);
    assert!(
        escaped_sleeps_stopped(&sleeps.0).await,
        "escaped sleeps still running: {:?}",
        sleeps.0
    );
}
