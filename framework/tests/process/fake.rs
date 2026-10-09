//! PAR-025: fakes that stop real execution, and the assertions on them.

use serial_test::serial;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;
use suprnova::{Process, ProcessError};

use crate::support::sh;

fn panics<R>(f: impl FnOnce() -> R) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        f();
    }))
    .is_err()
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
        Process::result("main\n")
            .error_output("warning")
            .exit_code(1),
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
    assert_eq!(
        other.output(),
        "anything else",
        "the first matching pattern wins"
    );
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
    assert_eq!(
        results.get("one").unwrap().as_ref().unwrap().output(),
        "ABC"
    );

    let piped = Process::pipe()
        .push(Process::command(["upper", "c"]))
        .push(Process::command(["upper", "d"]))
        .run()
        .await
        .unwrap();
    assert_eq!(piped.output(), "ABC");
    fake.assert_ran_count("upper a", 1);
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
    assert!(!panics(|| fake.assert_ran_count("ls -la", 2)));
    assert!(panics(|| fake.assert_ran_count("ls -la", 1)));
    assert!(!panics(|| fake.assert_ran_in_order(&[
        "ls -la",
        "ls -la",
        "sh -c echo hi"
    ])));
    assert!(panics(|| fake.assert_ran_in_order(&[
        "sh -c echo hi",
        "ls -la",
        "ls -la"
    ])));
    assert!(!panics(|| fake.assert_not_ran("rm -rf /")));
    assert!(panics(|| fake.assert_not_ran("ls -la")));
    assert!(panics(|| fake.assert_didnt_run("ls -la")));
    assert!(panics(|| fake.assert_nothing_ran()));
    assert!(!panics(
        || fake.assert_ran_with(|process| process.result.successful())
    ));
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

    let result = Process::shell("npm run build && npm test")
        .run()
        .await
        .unwrap();
    assert_eq!(result.output(), "built");
    fake.assert_ran("npm run build && npm test");
}

#[tokio::test]
#[serial]
async fn a_described_process_reports_its_id_and_replaced_output() {
    let fake = Process::fake();
    fake.when(
        "server *",
        Process::describe()
            .id(4242)
            .output("old")
            .replace_output("first\nsecond")
            .replace_error_output("warned"),
    );

    let process = Process::command(["server", "up"]).start().unwrap();
    assert_eq!(process.id(), Some(4242));
    process.signal(suprnova::Signal::Usr1).unwrap();
    assert!(process.has_received_signal(suprnova::Signal::Usr1));
    assert!(!process.has_received_signal(suprnova::Signal::Term));
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "first\nsecond\n");
    assert_eq!(result.error_output(), "warned\n");
}

#[tokio::test]
#[serial]
async fn a_sequence_takes_pushed_results_and_a_result_for_when_it_is_empty() {
    let fake = Process::fake();
    let sequence = Process::sequence([Process::result("one")])
        .push(Process::result("two"))
        .when_empty(Process::result("done").exit_code(9));
    assert!(!sequence.is_empty());
    fake.when("step *", sequence.clone());

    let step = || Process::command(["step", "x"]).run();
    assert_eq!(step().await.unwrap().output(), "one");
    assert_eq!(step().await.unwrap().output(), "two");
    assert!(sequence.is_empty());
    let after = step().await.unwrap();
    assert_eq!((after.output(), after.exit_code()), ("done", Some(9)));
}

// Review fixes.

#[tokio::test]
#[serial]
async fn a_quiet_faked_run_calls_no_callback_and_keeps_no_output() {
    let fake = Process::fake();
    fake.when("noisy *", Process::result("out").error_output("err"));
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0));
    let count = std::sync::Arc::clone(&calls);

    let result = Process::command(["noisy", "run"])
        .quietly()
        .run_with(move |_, _| *count.lock().unwrap() += 1)
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), 0, "as a real quiet run");
    assert_eq!((result.output(), result.error_output()), ("", ""));
}

#[tokio::test]
#[serial]
async fn closure_handlers_receive_the_pending_process_and_return_a_result() {
    let fake = Process::fake();
    fake.prevent_stray_processes();
    let dir = tempfile::tempdir().unwrap();
    let expected = dir.path().to_path_buf();
    fake.when("printf a b", move |pending: &suprnova::PendingProcess| {
        assert_eq!(
            pending.arguments(),
            Some(["printf".to_owned(), "a b".to_owned()].as_slice())
        );
        assert_eq!(pending.working_directory(), Some(expected.as_path()));
        assert_eq!(
            pending.environment(),
            &[
                ("VALUE".to_owned(), Some("set".to_owned())),
                ("PATH".to_owned(), None)
            ]
        );
        assert_eq!(pending.standard_input(), Some(b"input".as_slice()));
        Process::result("answer")
            .error_output("warning")
            .exit_code(7)
    });
    let result = Process::command(["printf", "a b"])
        .path(dir.path())
        .env("VALUE", "set")
        .env("PATH", None)
        .input("input")
        .run()
        .await
        .unwrap();
    assert_eq!(
        (result.output(), result.error_output(), result.exit_code()),
        ("answer", "warning", Some(7))
    );
    assert_eq!(result.command(), "printf 'a b'");
    fake.assert_ran_times("printf a b");
    assert!(Process::command(["other"]).run().await.is_err());
}

#[tokio::test]
#[serial]
async fn described_streams_replay_in_order_with_one_trailing_newline() {
    use std::sync::{Arc, Mutex};
    use suprnova::OutputKind::{Err, Out};
    let fake = Process::fake();
    fake.when(
        "mixed",
        Process::describe()
            .output("one\n\n")
            .error_output("two")
            .output("")
            .error_output("four\n")
            .runs_for(4),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let result = Process::command(["mixed"])
        .run_with(move |kind, chunk| {
            sink.lock().unwrap().push((kind, chunk.to_owned()));
        })
        .await
        .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            (Out, "one\n".to_owned()),
            (Err, "two\n".to_owned()),
            (Out, "\n".to_owned()),
            (Err, "four\n".to_owned())
        ]
    );
    assert_eq!(
        (result.output(), result.error_output()),
        ("one\n\n", "two\nfour\n")
    );
    fake.assert_ran_times("mixed");
    assert!(panics(|| fake.assert_ran_times("absent")));
    Process::command(["mixed"]).run().await.unwrap();
    assert!(panics(|| fake.assert_ran_times("mixed")));
    fake.assert_ran_count("mixed", 2);
}
