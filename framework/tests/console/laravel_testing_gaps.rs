//! The console surface of the Laravel testing gaps (PAR-177): up-front
//! output expectations, confirmations, exit codes of a command's own, and
//! the questions and expectations a run reports.
//!
//! One raw command, `testing-gaps:print`, does what its arguments say, in
//! order, so each test states the writes and questions it needs:
//!
//! - `out:<text>` writes `<text>` with `console::line`;
//! - `err:<text>` writes `<text>` with `console::error_line`;
//! - `ask:<question>` asks and fails with the error of `ask`;
//! - `swallow:<question>` asks and goes on whatever `ask` returned;
//! - `confirm:<question>` asks `confirm` with a `false` default and writes
//!   `confirmed <answer>`;
//! - `exit:<code>` ends with `FrameworkError::exit(code)`;
//! - `fail:<message>` ends with `FrameworkError::internal(message)`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use serial_test::serial;
use suprnova::console::{self, ConsoleRun};
use suprnova::{Exceptions, FrameworkError, command};

#[command(
    name = "testing-gaps:print",
    description = "Writes, asks and ends as its arguments say"
)]
async fn prints_as_told(args: Vec<String>) -> Result<(), FrameworkError> {
    for arg in args {
        let (step, value) = arg
            .split_once(':')
            .ok_or_else(|| FrameworkError::internal(format!("`{arg}` names no step")))?;
        match step {
            "out" => console::line(value),
            "err" => console::error_line(value),
            "ask" => {
                let answer = console::ask(value)?;
                console::line(format!("answered {answer}"));
            }
            "swallow" => {
                let _ignored = console::ask(value);
            }
            "confirm" => {
                let answer = console::confirm(value, false)?;
                console::line(format!("confirmed {answer}"));
            }
            "exit" => {
                let code: u8 = value
                    .parse()
                    .map_err(|_| FrameworkError::internal(format!("`{value}` is no exit code")))?;
                return Err(FrameworkError::exit(code));
            }
            "fail" => return Err(FrameworkError::internal(value)),
            other => return Err(FrameworkError::internal(format!("no step `{other}`"))),
        }
    }
    Ok(())
}

/// `console::test` on `testing-gaps:print` with `steps`.
fn print(steps: &[&str]) -> console::ConsoleTest {
    console::test(
        std::iter::once("testing-gaps:print".to_owned())
            .chain(steps.iter().map(|s| (*s).to_owned())),
    )
}

/// The message `assert` panicked with. Fails the test when it did not
/// panic.
#[track_caller]
fn panic_of(assert: impl FnOnce()) -> String {
    let payload = match catch_unwind(AssertUnwindSafe(assert)) {
        Ok(()) => panic!("the assertion passed and was expected to fail"),
        Err(payload) => payload,
    };
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        String::from("<a panic that carries no text>")
    }
}

/// `run` failed `assert_successful` with a message that holds every one of
/// `parts`.
#[track_caller]
fn assert_unsuccessful_naming(run: &ConsoleRun, parts: &[&str]) {
    let message = panic_of(|| {
        run.assert_successful();
    });
    for part in parts {
        assert!(
            message.contains(part),
            "the failure must name `{part}`:\n{message}"
        );
    }
}

// ---------------------------------------------------------------------------
// expects_output: exact lines, in order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expected_lines_pass_in_the_order_the_command_prints_them() {
    let run = print(&["out:a", "out:between", "out:b"])
        .expects_output("a")
        .expects_output("b")
        .run()
        .await;

    run.assert_successful();
    assert!(run.unmet_expectations().is_empty());
}

#[tokio::test]
async fn expected_lines_printed_in_another_order_fail_and_name_the_line() {
    let run = print(&["out:b", "out:a"])
        .expects_output("a")
        .expects_output("b")
        .run()
        .await;

    assert_eq!(run.exit_code(), 0, "the command itself succeeded");
    assert_eq!(run.unmet_expectations().len(), 1);
    assert!(
        run.unmet_expectations()[0].contains("`b`") && run.unmet_expectations()[0].contains("`a`"),
        "the report names the line and the one it had to follow: {:?}",
        run.unmet_expectations()
    );
    assert_unsuccessful_naming(&run, &["`b`"]);
}

#[tokio::test]
async fn an_expected_line_never_printed_is_reported() {
    let run = print(&["out:hello"])
        .expects_output("never printed")
        .expects_output("hel")
        .run()
        .await;

    assert_eq!(
        run.unmet_expectations().len(),
        2,
        "a line is matched whole, so `hel` is not `hello`: {:?}",
        run.unmet_expectations()
    );
    assert!(run.unmet_expectations()[0].contains("`never printed`"));
    assert!(run.unmet_expectations()[1].contains("`hel`"));
    assert_unsuccessful_naming(&run, &["`never printed`", "`hel`"]);
}

#[tokio::test]
async fn an_expected_line_matches_either_stream_and_each_line_of_a_write() {
    let run = print(&["out:one\ntwo", "err:careful", "out:done"])
        .expects_output("two")
        .expects_output("careful")
        .expects_output("done")
        .run()
        .await;

    run.assert_successful();
}

// ---------------------------------------------------------------------------
// expects_output_to_contain and doesnt_expect_output_to_contain
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expected_text_is_found_on_the_error_stream_too() {
    let run = print(&["out:nothing to see", "err:it went boom"])
        .expects_output_to_contain("boom")
        .run()
        .await;

    run.assert_successful();
    assert_eq!(run.output(), "nothing to see\n", "output() is unchanged");
    assert_eq!(run.errors(), "it went boom\n", "errors() is unchanged");
}

#[tokio::test]
async fn expected_text_never_spans_two_writes() {
    let two_writes = print(&["out:bo", "out:om"])
        .expects_output_to_contain("bo\nom")
        .expects_output_to_contain("boom")
        .run()
        .await;

    assert_eq!(
        two_writes.unmet_expectations().len(),
        2,
        "{:?}",
        two_writes.unmet_expectations()
    );
    assert_unsuccessful_naming(&two_writes, &["`boom`"]);

    let one_write = print(&["out:bo\nom"])
        .expects_output_to_contain("bo\nom")
        .run()
        .await;
    one_write.assert_successful();
}

#[tokio::test]
async fn text_the_test_does_not_expect_fails_on_either_stream() {
    for step in ["out:an x here", "err:an x here"] {
        let run = print(&[step])
            .doesnt_expect_output_to_contain("x")
            .run()
            .await;

        assert_eq!(run.unmet_expectations().len(), 1, "{step}");
        assert_unsuccessful_naming(&run, &["`x`"]);
    }

    let clean = print(&["out:nothing", "err:at all"])
        .doesnt_expect_output_to_contain("x")
        .run()
        .await;
    clean.assert_successful();

    let split = print(&["out:bo", "out:om"])
        .doesnt_expect_output_to_contain("bo\nom")
        .run()
        .await;
    split.assert_successful();
}

// ---------------------------------------------------------------------------
// expects_confirmation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_confirmation_is_answered_yes_or_no() {
    let yes = print(&["confirm:Delete every row?"])
        .expects_confirmation("Delete every row?", true)
        .expects_output("confirmed true")
        .run()
        .await;
    yes.assert_successful();

    let no = print(&["confirm:Delete every row?"])
        .expects_confirmation("Delete every row?", false)
        .expects_output("confirmed false")
        .run()
        .await;
    no.assert_successful();
}

#[tokio::test]
async fn a_confirmation_the_command_never_asks_fails_the_run() {
    let run = print(&["out:done"])
        .expects_confirmation("Delete every row?", true)
        .run()
        .await;

    assert_eq!(run.exit_code(), 0);
    assert_eq!(run.unasked_questions(), ["Delete every row?"]);
    assert_unsuccessful_naming(&run, &["`Delete every row?`"]);
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_command_ends_with_an_exit_code_of_its_own() {
    let run = print(&["out:checked", "exit:2"]).run().await;

    run.assert_exit_code(2)
        .assert_not_exit_code(1)
        .assert_not_exit_code(0)
        .assert_failed();
    assert_eq!(run.exit_code(), 2);
    assert_eq!(run.output(), "checked\n");
    assert_eq!(
        run.errors(),
        "",
        "an exit code is the outcome the command chose: nothing is printed for it"
    );
    assert_eq!(run.error().map(FrameworkError::exit_code), Some(2));
}

#[tokio::test]
async fn the_exit_code_assertions_name_both_codes() {
    let succeeded = print(&["out:done"]).run().await;
    let message = panic_of(|| {
        succeeded.assert_exit_code(2);
    });
    assert!(
        message.contains("exit code 0") && message.contains('2'),
        "{message}"
    );

    let failed = print(&["fail:broken"]).run().await;
    assert_eq!(failed.exit_code(), 1);
    let message = panic_of(|| {
        failed.assert_exit_code(2);
    });
    assert!(
        message.contains("exit code 1") && message.contains('2'),
        "{message}"
    );

    let two = print(&["exit:2"]).run().await;
    let message = panic_of(|| {
        two.assert_not_exit_code(2);
    });
    assert!(message.contains("exit code 2"), "{message}");
}

#[tokio::test]
async fn exit_code_0_is_a_success() {
    let run = print(&["out:done", "exit:0"]).run().await;

    run.assert_successful().assert_exit_code(0);
    assert!(run.error().is_none(), "{:?}", run.error());
    assert_eq!(run.errors(), "");
}

#[tokio::test]
async fn the_dispatcher_returns_the_code_the_console_binary_exits_with() {
    let argv = |step: &str| {
        vec![
            "console".to_owned(),
            "testing-gaps:print".to_owned(),
            step.to_owned(),
        ]
    };

    let own = console::dispatch_argv(argv("exit:3"))
        .await
        .expect_err("the command ended with its own code");
    assert_eq!(own.exit_code(), 3);

    let plain = console::dispatch_argv(argv("fail:broken"))
        .await
        .expect_err("the command failed");
    assert_eq!(plain.exit_code(), 1, "any other error ends with 1");

    let parse = console::dispatch_argv(vec!["console".to_owned(), "--nope".to_owned()])
        .await
        .expect_err("an argument the console does not take");
    assert_eq!(parse.exit_code(), 1);

    assert!(
        console::dispatch_argv(argv("exit:0")).await.is_ok(),
        "exit code 0 is a success"
    );
}

#[tokio::test]
#[serial]
async fn an_exit_code_is_not_reported_as_an_error() {
    Exceptions::reset();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Exceptions::reportable(move |error: &FrameworkError| {
        record
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(error.to_string());
    });

    print(&["exit:4"]).run().await.assert_exit_code(4);
    print(&["fail:testing-gaps reported failure"])
        .run()
        .await
        .assert_failed();
    let reported = seen.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Exceptions::reset();

    assert!(
        reported
            .iter()
            .any(|text| text.contains("testing-gaps reported failure")),
        "a failure is reported: {reported:?}"
    );
    assert!(
        !reported.iter().any(|text| text.contains("exit code 4")),
        "the exit code is not: {reported:?}"
    );
}

// ---------------------------------------------------------------------------
// Unexpected questions and unmet expectations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_question_the_test_did_not_expect_fails_every_assertion_on_the_run() {
    // The command goes on after `ask` refused it, so it ends with 0.
    let run = print(&["swallow:Delete every row?", "out:done"])
        .run()
        .await;

    assert_eq!(run.exit_code(), 0);
    assert_eq!(run.unexpected_questions(), ["Delete every row?"]);
    assert_unsuccessful_naming(&run, &["`Delete every row?`"]);
    for message in [
        panic_of(|| {
            run.assert_exit_code(0);
        }),
        panic_of(|| {
            run.assert_not_exit_code(1);
        }),
        panic_of(|| {
            run.assert_failed();
        }),
    ] {
        assert!(message.contains("`Delete every row?`"), "{message}");
    }
}

#[tokio::test]
async fn a_question_asked_out_of_order_names_the_question() {
    let run = print(&["ask:Second?", "ask:First?"])
        .expects_question("First?", "1")
        .expects_question("Second?", "2")
        .run()
        .await;

    assert_eq!(run.exit_code(), 1, "`ask` refused the question");
    assert_eq!(run.unexpected_questions(), ["Second?"]);
    assert_unsuccessful_naming(&run, &["`Second?`"]);
    let message = panic_of(|| {
        run.assert_failed();
    });
    assert!(message.contains("`Second?`"), "{message}");
}

#[tokio::test]
async fn every_unmet_expectation_is_reported_together() {
    let run = print(&["out:a boom"])
        .expects_question("Name?", "Ada")
        .expects_output("missing line")
        .expects_output_to_contain("missing text")
        .doesnt_expect_output_to_contain("boom")
        .run()
        .await;

    assert_eq!(run.exit_code(), 0);
    let unmet = run.unmet_expectations();
    assert_eq!(unmet.len(), 4, "{unmet:?}");
    for (entry, part) in unmet
        .iter()
        .zip(["`Name?`", "`missing line`", "`missing text`", "`boom`"])
    {
        assert!(entry.contains(part), "`{entry}` names {part}");
    }
    assert_unsuccessful_naming(
        &run,
        &["`Name?`", "`missing line`", "`missing text`", "`boom`"],
    );
    let message = panic_of(|| {
        run.assert_exit_code(0);
    });
    assert!(message.contains("`missing line`"), "{message}");
}

#[tokio::test]
async fn questions_stay_in_the_output_and_count_as_lines() {
    let run = print(&["ask:Name?"])
        .expects_question("Name?", "Ada")
        .expects_output("Name?")
        .expects_output("answered Ada")
        .run()
        .await;

    run.assert_successful();
    assert_eq!(run.output(), "Name?\nanswered Ada\n");
}
