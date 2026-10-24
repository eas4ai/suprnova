//! PAR-189 on the `process` driver: each task in a console process, this
//! test binary, started through the Process facade.

use crate::support::{CRAFTED_NAME, console_command, use_this_binary, wait_until};
use crate::tasks::{
    Answer, AnswerThenLogForgedOutcomes, Dashed, DriverReport, FORGED_OTHER_TOKEN, FORGED_OUTCOME,
    FORGED_UNTOKENED, FailWith, ForgeSuccessThenExit, Nap, NapThenEcho, PanicOnPurpose, PidThenNap,
    ReportDefaultDriver, WritePidThenNap,
};
use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::concurrency::{ProcessDriverOptions, Task};
use suprnova::{Concurrency, ConcurrentTask, Process};

/// Whether a process with this id is alive, by `kill -0`.
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[tokio::test]
#[serial]
async fn process_runs_two_one_second_tasks_side_by_side_and_answers_in_order() {
    use_this_binary();
    let started = Instant::now();
    let results = Concurrency::driver("process")
        .run([
            Task::new::<NapThenEcho>(Nap::new(1000, "first")),
            Task::new::<NapThenEcho>(Nap::new(1000, "second")),
        ])
        .await
        .expect("both tasks succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(2),
        "side by side: {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_secs(1),
        "each one sleeps: {elapsed:?}"
    );
    assert!(
        results
            .iter()
            .all(|result| result.last_progress().is_some() && result.idle_duration().is_some())
    );
    let values: Vec<String> = results
        .iter()
        .map(|result| result.get().expect("a string"))
        .collect();
    assert_eq!(values, ["first", "second"]);
}

#[tokio::test]
#[serial]
async fn process_run_named_answers_each_result_under_its_key() {
    use_this_binary();
    let results = Concurrency::driver("process")
        .run_named([
            ("users", Task::new::<NapThenEcho>(Nap::new(200, "u"))),
            ("orders", Task::new::<NapThenEcho>(Nap::new(0, "o"))),
        ])
        .await
        .expect("both tasks succeed");

    let keys: Vec<&str> = results.keys().map(String::as_str).collect();
    assert_eq!(keys, ["users", "orders"]);
    assert_eq!(results["users"].get::<String>().expect("u"), "u");
    assert_eq!(results["orders"].get::<String>().expect("o"), "o");
}

#[tokio::test]
#[serial]
async fn a_process_task_past_its_timeout_is_killed_and_the_error_names_it() {
    use_this_binary();
    let dir = tempfile::tempdir().expect("a directory");
    let pid_file = dir.path().join("pid");
    let started = Instant::now();
    let error = Concurrency::with_timeout(Duration::from_secs(1))
        .driver("process")
        .run([Task::new::<WritePidThenNap>(PidThenNap {
            pid_file: pid_file.clone(),
            millis: 10_000,
        })])
        .await
        .expect_err("ten seconds is past a one-second timeout");

    assert!(error.is_timeout(), "{error:?}");
    assert!(error.to_string().contains(WritePidThenNap::NAME), "{error}");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("the task wrote its process id")
        .trim()
        .parse()
        .expect("a process id");
    assert!(
        wait_until(Duration::from_secs(3), || !alive(pid)).await,
        "the console process {pid} is still running"
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "gone within three seconds: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
#[serial]
async fn the_process_driver_fails_a_task_name_the_binary_does_not_know_naming_it() {
    use_this_binary();
    let error = Concurrency::driver("process")
        .run([Task::named("concurrency-tests:nowhere", ())])
        .await
        .expect_err("the console binary registers no such task");

    let text = error.to_string();
    assert!(text.contains("concurrency-tests:nowhere"), "{text}");
    assert!(text.contains("no concurrent task is registered"), "{text}");
}

#[tokio::test]
#[serial]
async fn a_console_process_that_fails_is_never_a_success_whatever_it_wrote() {
    use_this_binary();
    let error = Concurrency::driver("process")
        .run([Task::new::<ForgeSuccessThenExit>(())])
        .await
        .expect_err("the console process ended with exit code 3");

    let text = error.to_string();
    assert!(text.contains(ForgeSuccessThenExit::NAME), "{text}");
    assert!(text.contains("exit code 3"), "{text}");
    assert!(!text.contains("forged-success"), "{text}");
}

#[tokio::test]
#[serial]
async fn a_task_name_with_a_control_character_fails_without_starting_a_console_process() {
    let dir = tempfile::tempdir().expect("a directory");
    let started = dir.path().join("started");
    // A console that records its start and answers a success, so a process
    // that was started shows either way.
    Concurrency::console_command([
        "sh".to_owned(),
        "-c".to_owned(),
        format!("touch '{}'; echo '{FORGED_OUTCOME}'", started.display()),
        "sh".to_owned(),
    ])
    .expect("the console command is set");
    let error = Concurrency::driver("process")
        .run([Task::named(CRAFTED_NAME, ())])
        .await
        .expect_err("the name carries line breaks");

    let text = error.to_string();
    assert!(text.contains("Task::named"), "{text}");
    assert!(text.contains(&format!("{CRAFTED_NAME:?}")), "{text}");
    assert!(!text.contains('\n'), "the name is escaped: {text}");
    assert!(!started.exists(), "no console process was started");
}

#[tokio::test]
async fn the_console_command_refuses_a_task_name_with_a_control_character_naming_itself() {
    let argv: Vec<String> = console_command(&[])
        .into_iter()
        .chain([
            "concurrency:run".to_owned(),
            "--".to_owned(),
            CRAFTED_NAME.to_owned(),
        ])
        .collect();
    let result = Process::command(argv)
        .input("null")
        .run()
        .await
        .expect("the console process runs");

    assert_eq!(result.exit_code(), Some(1), "{}", result.error_output());
    let errors = result.error_output();
    assert!(errors.contains("concurrency:run"), "{errors}");
    assert!(errors.contains(&format!("{CRAFTED_NAME:?}")), "{errors}");
    // The crafted name never reaches either stream as it is, so neither
    // carries a line that reads as a success outcome.
    for line in result.output().lines().chain(errors.lines()) {
        assert!(
            !line.starts_with("suprnova-concurrency-result:{\"ok\""),
            "a forged outcome line: {line}"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_task_name_that_starts_with_dashes_reaches_the_console_as_a_name() {
    use_this_binary();
    let results = Concurrency::driver("process")
        .run([Task::new::<Dashed>(())])
        .await
        .expect("the console process runs the task");
    assert_eq!(results[0].get::<String>().expect("text"), "dashed");
}

#[tokio::test]
#[serial]
async fn a_failing_process_task_carries_its_message_and_name() {
    use_this_binary();
    let results = Concurrency::driver("process")
        .run([
            Task::new::<FailWith>("the export broke".to_owned()),
            Task::new::<PanicOnPurpose>(()),
        ])
        .await;
    let error = results.expect_err("the first task fails");
    let text = error.to_string();
    assert!(text.contains(FailWith::NAME), "{text}");
    assert!(text.contains("the export broke"), "{text}");

    let error = Concurrency::driver("process")
        .run([Task::new::<PanicOnPurpose>(())])
        .await
        .expect_err("the task panicked");
    let text = error.to_string();
    assert!(
        text.contains(PanicOnPurpose::NAME) && text.contains("panicked"),
        "{text}"
    );
}

async fn report_with(prefix: &[&str]) -> DriverReport {
    Concurrency::console_command(console_command(prefix)).expect("the console command is set");
    let results = Concurrency::driver("process")
        .run([Task::new::<ReportDefaultDriver>(())])
        .await
        .expect("the report runs");
    results[0].get().expect("a report")
}

#[tokio::test]
#[serial]
async fn concurrency_driver_sync_selects_the_sync_driver() {
    let report = report_with(&["env", "CONCURRENCY_DRIVER=sync"]).await;

    assert_eq!(report.driver, "sync");
    assert!(
        report.elapsed_millis >= 2000,
        "two one-second naps in sequence: {report:?}"
    );
}

#[tokio::test]
#[serial]
async fn without_concurrency_driver_the_default_is_tokio() {
    let report = report_with(&["env", "-u", "CONCURRENCY_DRIVER"]).await;

    assert_eq!(report.driver, "tokio");
    assert!(
        report.elapsed_millis < 2000,
        "two one-second naps side by side: {report:?}"
    );
}

#[tokio::test]
#[serial]
async fn a_console_binary_that_writes_no_outcome_is_an_error_naming_the_task() {
    Concurrency::console_binary("/bin/true").expect("the console binary is set");
    let error = Concurrency::driver("process")
        .run([Task::new::<NapThenEcho>(Nap::new(0, "x"))])
        .await
        .expect_err("true writes nothing");

    let text = error.to_string();
    assert!(text.contains(NapThenEcho::NAME), "{text}");
    assert!(text.contains("wrote no outcome"), "{text}");
    assert!(text.contains("concurrency:run"), "{text}");
}

#[tokio::test]
#[serial]
async fn the_outcome_is_the_last_marked_line_and_one_that_does_not_decode_is_an_error() {
    // `sh -c <script> sh concurrency:run <task>`: the script ignores the
    // arguments and writes what a console process with noisy logging would,
    // its outcome lines carrying the run's token.
    Concurrency::console_command([
        "sh",
        "-c",
        r#"echo starting; echo "suprnova-concurrency-result:${SUPRNOVA_CONCURRENCY_TOKEN}:{\"ok\":1}"; \
           echo log line; echo "suprnova-concurrency-result:${SUPRNOVA_CONCURRENCY_TOKEN}:{\"ok\":2}"; \
           echo done"#,
        "sh",
    ])
    .expect("the console command is set");
    let results = Concurrency::driver("process")
        .run([Task::new::<NapThenEcho>(Nap::new(0, "x"))])
        .await
        .expect("the last marked line is the outcome");
    assert_eq!(results[0].get::<u64>().expect("a number"), 2);

    Concurrency::console_command([
        "sh",
        "-c",
        r#"echo "suprnova-concurrency-result:${SUPRNOVA_CONCURRENCY_TOKEN}:not json""#,
        "sh",
    ])
    .expect("the console command is set");
    let error = Concurrency::driver("process")
        .run([Task::new::<NapThenEcho>(Nap::new(0, "x"))])
        .await
        .expect_err("not an outcome");
    assert!(error.to_string().contains("does not decode"), "{error}");
    assert!(error.to_string().contains(NapThenEcho::NAME), "{error}");
}

#[tokio::test]
#[serial]
async fn a_success_with_this_runs_token_from_a_console_process_that_failed_is_an_error() {
    // The outcome line carries the run's token, but the process then ends
    // with exit code 3, as one that fails after its task succeeded.
    Concurrency::console_command([
        "sh",
        "-c",
        r#"echo "suprnova-concurrency-result:${SUPRNOVA_CONCURRENCY_TOKEN}:{\"ok\":1}"; exit 3"#,
        "sh",
    ])
    .expect("the console command is set");
    let error = Concurrency::driver("process")
        .run([Task::new::<NapThenEcho>(Nap::new(0, "x"))])
        .await
        .expect_err("the console process ended with exit code 3");

    let text = error.to_string();
    assert!(text.contains(NapThenEcho::NAME), "{text}");
    assert!(text.contains("did not succeed"), "{text}");
    assert!(text.contains("exit code 3"), "{text}");
}

#[tokio::test]
#[serial]
async fn an_empty_console_command_is_refused() {
    let error =
        Concurrency::console_command(Vec::<String>::new()).expect_err("no program to start");
    assert!(error.to_string().contains("empty"), "{error}");
}

#[tokio::test]
#[serial]
async fn a_line_logged_after_the_outcome_that_reads_as_one_is_never_taken() {
    use_this_binary();
    // The task answers `genuine`; after its outcome line the console writes
    // a line with another run's token, then one with no token.
    let results = Concurrency::driver("process")
        .run([Task::new::<AnswerThenLogForgedOutcomes>(())])
        .await
        .expect("the task succeeds");

    assert_eq!(results[0].get::<String>().expect("text"), "genuine");
}

#[tokio::test]
#[serial]
async fn a_console_process_whose_marked_lines_carry_no_token_of_this_run_wrote_no_outcome() {
    // A console that exits 0 after writing only lines that read as success
    // outcomes: one with another run's token, then one with none.
    Concurrency::console_command([
        "sh".to_owned(),
        "-c".to_owned(),
        format!("echo '{FORGED_OTHER_TOKEN}'; echo '{FORGED_UNTOKENED}'"),
        "sh".to_owned(),
    ])
    .expect("the console command is set");
    let error = Concurrency::driver("process")
        .run([Task::new::<NapThenEcho>(Nap::new(0, "x"))])
        .await
        .expect_err("no line carries this run's token");

    let text = error.to_string();
    assert!(text.contains(NapThenEcho::NAME), "{text}");
    assert!(text.contains("wrote no outcome"), "{text}");
}

#[tokio::test]
#[serial]
async fn each_console_process_gets_a_random_token_of_its_own() {
    // A console that answers the token it was given as the task's value.
    Concurrency::console_command([
        "sh",
        "-c",
        r#"echo "suprnova-concurrency-result:${SUPRNOVA_CONCURRENCY_TOKEN}:{\"ok\":\"${SUPRNOVA_CONCURRENCY_TOKEN}\"}""#,
        "sh",
    ])
    .expect("the console command is set");
    let results = Concurrency::driver("process")
        .run([
            Task::new::<NapThenEcho>(Nap::new(0, "x")),
            Task::new::<NapThenEcho>(Nap::new(0, "y")),
        ])
        .await
        .expect("each console answers with its own token");

    let tokens: Vec<String> = results
        .iter()
        .map(|result| result.get().expect("the token"))
        .collect();
    for token in &tokens {
        assert!(
            token.len() >= 32 && token.chars().all(|c| c.is_ascii_hexdigit()),
            "at least 128 bits as hex: {token:?}"
        );
    }
    assert_ne!(tokens[0], tokens[1], "one token per console process");
}

/// Run `concurrency:run` for the `answer` task by hand, with this binary as
/// the console, after `prefix`, and answer its standard output.
async fn run_answer_by_hand(prefix: &[&str]) -> String {
    let argv: Vec<String> = console_command(prefix)
        .into_iter()
        .chain([
            "concurrency:run".to_owned(),
            "--".to_owned(),
            Answer::NAME.to_owned(),
        ])
        .collect();
    let result = Process::command(argv)
        .input("null")
        .run()
        .await
        .expect("the console process runs");
    assert_eq!(result.exit_code(), Some(0), "{}", result.error_output());
    result.output().to_owned()
}

#[tokio::test]
async fn the_console_command_writes_the_token_it_is_given_and_a_plain_line_without_one() {
    let output = run_answer_by_hand(&["env", "SUPRNOVA_CONCURRENCY_TOKEN=0123abcd"]).await;
    assert!(
        output
            .lines()
            .any(|line| line == "suprnova-concurrency-result:0123abcd:{\"ok\":42}"),
        "{output}"
    );

    let output = run_answer_by_hand(&["env", "-u", "SUPRNOVA_CONCURRENCY_TOKEN"]).await;
    assert!(
        output
            .lines()
            .any(|line| line == "suprnova-concurrency-result:{\"ok\":42}"),
        "{output}"
    );
}

#[tokio::test]
#[serial]
async fn process_driver_honors_a_shared_caller_concurrency_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        r#"mkdir '{0}/active-'"$3"; while [ ! -f '{0}/release' ]; do sleep 0.02; done; echo "suprnova-concurrency-result:$SUPRNOVA_CONCURRENCY_TOKEN:{{\"ok\":42}}""#,
        dir.path().display()
    );
    Concurrency::console_command(["sh", "-c", &script, "sh"]).unwrap();
    let options = ProcessDriverOptions::default().max_concurrency(3);
    let first = Concurrency::driver("process")
        .process_options(options.clone())
        .run((0..6).map(|i| Task::named(format!("job-{i}"), ())));
    let second = Concurrency::driver("process")
        .process_options(options)
        .run((6..12).map(|i| Task::named(format!("job-{i}"), ())));
    let runs = tokio::spawn(async move { tokio::join!(first, second) });
    let count = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("active-"))
            .count()
    };
    assert!(wait_until(Duration::from_secs(5), || count() >= 3).await);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = count();
    std::fs::write(dir.path().join("release"), "go").unwrap();
    let (first, second) = runs.await.unwrap();
    assert_eq!(
        started, 3,
        "the caller selected three children across both calls"
    );
    assert_eq!(first.unwrap().len() + second.unwrap().len(), 12);
}

#[tokio::test(start_paused = true)]
#[serial]
async fn process_driver_has_no_default_deadline() {
    Concurrency::console_command([
        "sh",
        "-c",
        r#"sleep 0.2; echo "suprnova-concurrency-result:$SUPRNOVA_CONCURRENCY_TOKEN:{\"ok\":42}""#,
        "sh",
    ])
    .unwrap();
    let run = tokio::spawn(async {
        Concurrency::driver("process")
            .run([Task::named("slow-task", ())])
            .await
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(61)).await;
    assert_eq!(run.await.unwrap().unwrap()[0].get::<u64>().unwrap(), 42);
}

#[tokio::test]
#[serial]
async fn process_driver_output_limit_error_names_the_task() {
    Concurrency::console_command(["sh", "-c", "head -c 8388609 /dev/zero", "sh"]).unwrap();
    let error = Concurrency::driver("process")
        .process_options(ProcessDriverOptions::default().max_stream_output_bytes(1024))
        .run([Task::named("noisy-task", ())])
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("output limit"), "{error}");
    assert!(error.to_string().contains("noisy-task"), "{error}");
}

#[tokio::test]
#[serial]
async fn process_driver_hang_window_names_the_task_and_idle_time() {
    Concurrency::console_command(["sh", "-c", "sleep 30", "sh"]).unwrap();
    let error = Concurrency::driver("process")
        .hang_after(Duration::from_millis(500))
        .run([Task::named("hung-task", ())])
        .await
        .expect_err("no progress within the caller's hang window");
    let text = error.to_string();
    assert!(
        text.contains("hung-task") && text.contains("hung") && text.contains("idle for"),
        "{text}"
    );
}

#[tokio::test]
#[serial]
async fn process_driver_has_no_default_concurrency_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    Concurrency::console_command([
        "sh".to_owned(), "-c".to_owned(),
        format!(r#"touch '{0}/'"$3"; while [ ! -f '{0}/release' ]; do sleep 0.02; done; echo "suprnova-concurrency-result:$SUPRNOVA_CONCURRENCY_TOKEN:{{\"ok\":42}}""#, dir.path().display()),
        "sh".to_owned(),
    ]).unwrap();
    let run = tokio::spawn(async {
        Concurrency::driver("process")
            .run((0..12).map(|i| Task::named(format!("job-{i}"), ())))
            .await
    });
    let all_started = wait_until(Duration::from_secs(5), || {
        std::fs::read_dir(dir.path()).unwrap().count() == 12
    })
    .await;
    std::fs::write(dir.path().join("release"), "go").unwrap();
    assert_eq!(run.await.unwrap().unwrap().len(), 12);
    assert!(all_started, "all twelve children start before any exits");
}

#[tokio::test]
#[serial]
async fn process_driver_output_progress_survives_the_hang_window() {
    Concurrency::console_command([
        "sh", "-c",
        r#"i=0; while [ $i -lt 20 ]; do echo working; sleep 0.1; i=$((i+1)); done; echo "suprnova-concurrency-result:$SUPRNOVA_CONCURRENCY_TOKEN:{\"ok\":42}""#,
        "sh",
    ]).unwrap();
    let results = Concurrency::driver("process")
        .process_options(ProcessDriverOptions::default().hang_after(Duration::from_millis(500)))
        .run([Task::named("working-task", ())])
        .await
        .unwrap();
    assert_eq!(results[0].get::<u64>().unwrap(), 42);
    assert!(results[0].last_progress().is_some());
    assert!(results[0].idle_duration().unwrap() < Duration::from_millis(500));
}
