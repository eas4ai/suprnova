//! PAR-025: fakes that stop real execution, and the assertions on them.

use serial_test::serial;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;
use suprnova::{Process, ProcessError};

use crate::support::sh;

fn panics(f: impl FnOnce()) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

#[tokio::test]
#[serial]
async fn a_faked_command_does_not_run() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("created");
    let fake = Process::fake();

    let result = Process::command(["touch", marker.to_str().unwrap()])
        .run()
        .await
        .expect("an unmatched command gets an empty success");
    assert!(!marker.exists(), "nothing ran");
    assert!(result.successful());
    assert_eq!(result.output(), "");
    fake.assert_ran(&format!("touch {}", marker.display()));
}

#[tokio::test]
#[serial]
async fn a_matching_pattern_gets_its_result() {
    let fake = Process::fake();
    fake.when(
        "git *",
        Process::result("main\n").error_output("warning").exit_code(1),
    );
    fake.when("*", Process::result("anything else"));

    let git = Process::command(["git", "branch", "--show-current"])
        .run()
        .await
        .unwrap();
    assert_eq!(git.output(), "main\n");
    assert_eq!(git.error_output(), "warning");
    assert_eq!(git.exit_code(), Some(1));

    let other = Process::command(["ls"]).run().await.unwrap();
    assert_eq!(other.output(), "anything else", "the first matching pattern wins");
}

#[tokio::test]
#[serial]
async fn prevent_stray_processes_refuses_an_unmatched_command() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("created");
    let fake = Process::fake();
    fake.when("git *", Process::result("ok"));
    fake.prevent_stray_processes();

    let error = Process::command(["touch", marker.to_str().unwrap()])
        .run()
        .await
        .expect_err("a stray process is refused");
    assert!(matches!(error, ProcessError::Stray { .. }), "{error:?}");
    assert!(error.to_string().contains("touch"), "{error}");
    assert!(!marker.exists());
    assert!(Process::command(["git", "status"]).run().await.is_ok());
}

#[tokio::test]
#[serial]
async fn a_sequence_answers_in_turn() {
    let fake = Process::fake();
    fake.when(
        "deploy *",
        Process::sequence([Process::result("first"), Process::result("second")]),
    );

    let run = || Process::command(["deploy", "now"]).run();
    assert_eq!(run().await.unwrap().output(), "first");
    assert_eq!(run().await.unwrap().output(), "second");
    assert!(run().await.is_err(), "an exhausted sequence is an error");

    fake.when(
        "build *",
        Process::sequence([Process::result("only")]).dont_fail_when_empty(),
    );
    let build = || Process::command(["build", "it"]).run();
    assert_eq!(build().await.unwrap().output(), "only");
    let after = build().await.expect("an empty success once exhausted");
    assert!(after.successful());
    assert_eq!(after.output(), "");
}

#[tokio::test]
#[serial]
async fn a_described_process_runs_for_its_count_and_shows_its_output() {
    let fake = Process::fake();
    fake.when(
        "worker *",
        Process::describe()
            .output("line one")
            .output("line two")
            .error_output("careful")
            .exit_code(3)
            .runs_for(2),
    );

    let mut process = Process::command(["worker", "go"]).start().unwrap();
    assert!(process.running());
    assert!(process.running());
    assert!(!process.running(), "done after two checks");
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "line one\nline two\n");
    assert_eq!(result.error_output(), "careful\n");
    assert_eq!(result.exit_code(), Some(3));
}

#[tokio::test]
#[serial]
async fn pools_and_pipes_are_faked_and_recorded() {
    let fake = Process::fake();
    fake.when("upper *", Process::result("ABC"));

    let results = Process::pool()
        .add("one", Process::command(["upper", "a"]))
        .add("two", Process::command(["upper", "b"]))
        .run()
        .await;
    assert_eq!(results.get("one").unwrap().as_ref().unwrap().output(), "ABC");

    let piped = Process::pipe()
        .push(Process::command(["upper", "c"]))
        .push(Process::command(["upper", "d"]))
        .run()
        .await
        .unwrap();
    assert_eq!(piped.output(), "ABC");
    fake.assert_ran_times("upper a", 1);
    fake.assert_ran_with(|process| process.command == "upper d" && process.input == b"ABC");
}

#[tokio::test]
#[serial]
async fn the_assertions_fail_on_a_record_that_does_not_match() {
    let fake = Process::fake();
    assert!(!panics(|| fake.assert_nothing_ran()));
    assert!(panics(|| fake.assert_ran("ls")), "nothing ran yet");

    Process::command(["ls", "-la"]).run().await.unwrap();
    Process::command(["ls", "-la"]).run().await.unwrap();
    Process::command(sh("echo hi")).run().await.unwrap();

    assert!(!panics(|| fake.assert_ran("ls -la")));
    assert!(panics(|| fake.assert_ran("ls")), "an exact command line");
    assert!(!panics(|| fake.assert_ran_times("ls -la", 2)));
    assert!(panics(|| fake.assert_ran_times("ls -la", 1)));
    assert!(!panics(|| fake.assert_ran_in_order(&["ls -la", "ls -la", "sh -c echo hi"])));
    assert!(panics(|| fake.assert_ran_in_order(&["sh -c echo hi", "ls -la", "ls -la"])));
    assert!(!panics(|| fake.assert_not_ran("rm -rf /")));
    assert!(panics(|| fake.assert_not_ran("ls -la")));
    assert!(panics(|| fake.assert_didnt_run("ls -la")));
    assert!(panics(|| fake.assert_nothing_ran()));
    assert!(!panics(|| fake.assert_ran_with(|process| process.result.successful())));
}

#[tokio::test]
#[serial]
async fn the_fake_ends_with_its_guard() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("created");
    {
        let _fake = Process::fake();
        Process::command(["touch", marker.to_str().unwrap()])
            .run()
            .await
            .unwrap();
        assert!(!marker.exists());
    }
    Process::command(["touch", marker.to_str().unwrap()])
        .timeout(Duration::from_secs(5))
        .run()
        .await
        .unwrap();
    assert!(marker.exists(), "real processes run again");
}

#[tokio::test]
#[serial]
async fn a_shell_line_is_matched_and_recorded_as_given() {
    let fake = Process::fake();
    fake.when("npm run *", Process::result("built"));

    let result = Process::shell("npm run build && npm test").run().await.unwrap();
    assert_eq!(result.output(), "built");
    fake.assert_ran("npm run build && npm test");
}
