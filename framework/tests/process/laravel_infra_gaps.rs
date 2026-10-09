//! The Laravel infrastructure gaps of the process fake: the recorded
//! argument list and quoted command line, and a started fake that reveals
//! its output a line at a time (PAR-154).
//!
//! The fake is process-global while its guard lives, so every test is
//! `#[serial]`.

use serial_test::serial;
use std::panic::{AssertUnwindSafe, catch_unwind};
use suprnova::Process;

fn panics<R>(f: impl FnOnce() -> R) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        f();
    }))
    .is_err()
}

#[tokio::test]
#[serial]
async fn assert_ran_args_compares_the_argument_list_exactly() {
    let fake = Process::fake();

    Process::command(["printf", "a", "b"]).run().await.unwrap();

    assert!(
        panics(|| fake.assert_ran_args(&["printf", "a b"])),
        "two arguments are not the one argument `a b`"
    );
    fake.assert_ran_args(&["printf", "a", "b"]);
    fake.assert_ran("printf a b");
    fake.assert_ran_command_line("printf a b");
    assert!(panics(|| fake.assert_ran_command_line("printf 'a b'")));

    let recorded = fake.recorded();
    assert_eq!(
        recorded[0].args,
        Some(vec!["printf".to_owned(), "a".to_owned(), "b".to_owned()])
    );
}

#[tokio::test]
#[serial]
async fn a_recorded_process_keeps_its_arguments_and_its_quoted_line() {
    let fake = Process::fake();

    Process::command(["printf", "a b"]).run().await.unwrap();

    let recorded = fake.recorded();
    assert_eq!(
        recorded[0].args,
        Some(vec!["printf".to_owned(), "a b".to_owned()])
    );
    assert_eq!(recorded[0].command_line, "printf 'a b'");
    assert_eq!(
        recorded[0].command, "printf a b",
        "the patterns keep matching the space join"
    );
    assert_eq!(recorded[0].result.command(), "printf 'a b'");

    fake.assert_ran_args(&["printf", "a b"]);
    fake.assert_ran_command_line("printf 'a b'");
    fake.assert_ran("printf a b");
    assert!(panics(|| fake.assert_ran_args(&["printf", "a", "b"])));
    assert!(panics(|| fake.assert_ran_command_line("printf a b")));
}

#[tokio::test]
#[serial]
async fn a_shell_line_records_no_argument_list() {
    let fake = Process::fake();

    Process::shell("echo 'hi there' | wc -c")
        .run()
        .await
        .unwrap();

    let recorded = fake.recorded();
    assert_eq!(recorded[0].args, None);
    assert_eq!(recorded[0].command_line, "echo 'hi there' | wc -c");
    fake.assert_ran_command_line("echo 'hi there' | wc -c");
    assert!(
        panics(|| fake.assert_ran_args(&["echo", "'hi there'", "|", "wc", "-c"])),
        "a shell line has no argument list to compare"
    );
}

#[tokio::test]
#[serial]
async fn output_reveals_one_more_line_per_call() {
    let fake = Process::fake();
    fake.when("worker", Process::describe().output("one").output("two"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.output(), "one\n");
    assert_eq!(process.output(), "one\ntwo\n");
    assert_eq!(process.output(), "one\ntwo\n", "nothing more to reveal");
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "one\ntwo\n");
}

#[tokio::test]
#[serial]
async fn latest_output_answers_each_line_once_and_then_nothing() {
    let fake = Process::fake();
    fake.when("worker", Process::describe().output("one").output("two"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "one\n");
    assert_eq!(process.latest_output(), "two\n");
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.output(), "one\ntwo\n");
}

#[tokio::test]
#[serial]
async fn output_and_latest_output_share_one_counter_per_stream() {
    let fake = Process::fake();
    fake.when(
        "worker",
        Process::describe()
            .output("one")
            .error_output("bad")
            .output("two")
            .error_output("worse")
            .output("three"),
    );

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "one\n");
    assert_eq!(
        process.output(),
        "one\ntwo\n",
        "output reveals the next line"
    );
    assert_eq!(process.latest_output(), "three\n");

    assert_eq!(
        process.error_output(),
        "bad\n",
        "the error stream has its own counter"
    );
    assert_eq!(process.latest_error_output(), "worse\n");
    assert_eq!(process.latest_error_output(), "");
    assert_eq!(process.error_output(), "bad\nworse\n");

    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "one\ntwo\nthree\n");
    assert_eq!(result.error_output(), "bad\nworse\n");
}

#[tokio::test]
#[serial]
async fn lines_running_showed_count_as_revealed() {
    let fake = Process::fake();
    fake.when(
        "worker",
        Process::describe()
            .output("one")
            .output("two")
            .output("three")
            .runs_for(5),
    );

    let mut process = Process::command(["worker"]).start().unwrap();
    assert!(process.running(), "running shows the line `one`");
    assert_eq!(process.latest_output(), "two\n");
    assert_eq!(process.output(), "one\ntwo\nthree\n");
    assert_eq!(process.latest_output(), "");
}

#[tokio::test]
#[serial]
async fn every_line_is_revealed_once_running_answers_false() {
    let fake = Process::fake();
    fake.when(
        "worker",
        Process::describe()
            .output("one")
            .output("two")
            .error_output("err")
            .runs_for(0),
    );

    let mut process = Process::command(["worker"]).start().unwrap();
    assert!(!process.running());
    assert_eq!(process.output(), "one\ntwo\n");
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.error_output(), "err\n");
    assert_eq!(process.latest_error_output(), "");
}

#[tokio::test]
#[serial]
async fn a_quiet_started_fake_reveals_nothing() {
    let fake = Process::fake();
    fake.when("worker", Process::describe().output("one").output("two"));

    let process = Process::command(["worker"]).quietly().start().unwrap();
    assert_eq!(process.output(), "");
    assert_eq!(process.latest_output(), "");
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "");
}

#[tokio::test]
#[serial]
async fn a_fixed_result_reveals_its_whole_output_as_one_line() {
    let fake = Process::fake();
    fake.when("worker", Process::result("all of it\n"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "all of it\n");
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.output(), "all of it\n");
}
