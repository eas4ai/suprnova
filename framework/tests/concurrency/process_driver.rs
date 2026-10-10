//! PAR-189 on the `process` driver: each task in a console process, this
//! test binary, started through the Process facade.

use crate::support::{CRAFTED_NAME, console_command, use_this_binary, wait_until};
use crate::tasks::{
    Dashed, DriverReport, FORGED_OUTCOME, FailWith, ForgeSuccessThenExit, Nap, NapThenEcho,
    PanicOnPurpose, PidThenNap, ReportDefaultDriver, WritePidThenNap,
};
use serial_test::serial;
use std::time::{Duration, Instant};
use suprnova::concurrency::Task;
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
    // arguments and writes what a console process with noisy logging would.
    Concurrency::console_command([
        "sh",
        "-c",
        "echo starting; echo 'suprnova-concurrency-result:{\"ok\":1}'; echo log line; \
         echo 'suprnova-concurrency-result:{\"ok\":2}'; echo done",
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
        "echo 'suprnova-concurrency-result:not json'",
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
async fn an_empty_console_command_is_refused() {
    let error =
        Concurrency::console_command(Vec::<String>::new()).expect_err("no program to start");
    assert!(error.to_string().contains("empty"), "{error}");
}
