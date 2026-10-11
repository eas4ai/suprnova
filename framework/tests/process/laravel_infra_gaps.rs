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

#[tokio::test]
#[serial]
async fn a_fixed_result_reveals_one_more_line_per_output_call() {
    let fake = Process::fake();
    fake.when("worker", Process::result("one\ntwo\n"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(
        process.output(),
        "one\n",
        "the first call reveals the first line only"
    );
    assert_eq!(process.output(), "one\ntwo\n");
    assert_eq!(process.output(), "one\ntwo\n", "nothing more to reveal");
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "one\ntwo\n");

    let ran = Process::command(["worker"]).run().await.unwrap();
    assert_eq!(ran.output(), "one\ntwo\n", "a run keeps the whole output");
    assert_eq!(fake.recorded()[0].result.output(), "one\ntwo\n");
}

#[tokio::test]
#[serial]
async fn a_fixed_result_answers_latest_output_a_line_at_a_time() {
    let fake = Process::fake();
    fake.when("worker", Process::result("one\ntwo\n"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "one\n");
    assert_eq!(process.latest_output(), "two\n");
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.output(), "one\ntwo\n");
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "one\ntwo\n");
}

#[tokio::test]
#[serial]
async fn a_fixed_result_reveals_its_error_output_a_line_at_a_time() {
    let fake = Process::fake();
    fake.when("worker", Process::result("").error_output("bad\nworse\n"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_error_output(), "bad\n");
    assert_eq!(process.latest_error_output(), "worse\n");
    assert_eq!(process.latest_error_output(), "");
    assert_eq!(process.error_output(), "bad\nworse\n");
    let result = process.wait().await.unwrap();
    assert_eq!(result.error_output(), "bad\nworse\n");

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.error_output(), "bad\n");
    assert_eq!(process.error_output(), "bad\nworse\n");
    assert_eq!(process.output(), "", "the result has no standard output");
}

#[tokio::test]
#[serial]
async fn a_fixed_result_keeps_blank_lines_and_a_last_line_without_a_newline() {
    let fake = Process::fake();
    fake.when("worker", Process::result("one\n\ntwo"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "one\n");
    assert_eq!(process.latest_output(), "\n", "a blank line is a line");
    assert_eq!(
        process.latest_output(),
        "two",
        "the last line gets no newline it did not have"
    );
    assert_eq!(process.latest_output(), "");
    assert_eq!(
        process.output(),
        "one\n\ntwo",
        "the revealed lines reproduce the output byte for byte"
    );
    let result = process.wait().await.unwrap();
    assert_eq!(result.output(), "one\n\ntwo");
}

#[tokio::test]
#[serial]
async fn a_described_line_holding_a_newline_reveals_each_line_in_turn() {
    let fake = Process::fake();
    fake.when(
        "worker",
        Process::describe().output("a\nb").error_output("x\ny"),
    );
    fake.when("replaced", Process::describe().replace_output("c\nd\n"));

    let process = Process::command(["worker"]).start().unwrap();
    assert_eq!(process.latest_output(), "a\n");
    assert_eq!(
        process.latest_output(),
        "b\n",
        "a described line ends with one newline"
    );
    assert_eq!(process.latest_output(), "");
    assert_eq!(process.latest_error_output(), "x\n");
    assert_eq!(process.error_output(), "x\ny\n");
    let result = process.wait().await.unwrap();
    assert_eq!(
        (result.output(), result.error_output()),
        ("a\nb\n", "x\ny\n")
    );

    let replaced = Process::command(["replaced"]).start().unwrap();
    assert_eq!(replaced.output(), "c\n");
    assert_eq!(replaced.output(), "c\nd\n");
    let result = replaced.wait().await.unwrap();
    assert_eq!(result.output(), "c\nd\n");
}

#[tokio::test]
#[serial]
async fn running_shows_one_line_of_a_multi_line_description_at_a_time() {
    let fake = Process::fake();
    fake.when(
        "worker",
        Process::describe().output("one\ntwo\nthree").runs_for(5),
    );

    let mut process = Process::command(["worker"]).start().unwrap();
    assert!(process.running(), "running shows the line `one`");
    assert_eq!(process.latest_output(), "two\n");
    assert_eq!(process.output(), "one\ntwo\nthree\n");
    assert_eq!(process.latest_output(), "");
}
