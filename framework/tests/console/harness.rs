//! `console::test`: run a command in a test, read what it printed, and
//! answer what it asks.
//!
//! The commands here print with `console::line` and `console::error_line`
//! and ask with `console::ask` and `console::confirm`. The run goes
//! through the dispatcher the console binary uses, so what the console
//! prints itself, help and errors, is read the same way.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use clap::Parser;
// `db:seed` is driven below with the seeder registry reset, which only the
// `testing` feature has.
#[cfg(feature = "testing")]
use serial_test::serial;
#[cfg(feature = "testing")]
use suprnova::seed::{self, Seeder};
use suprnova::{Command, FrameworkError, TypedCommand, console};

#[derive(Parser, Command, Debug)]
#[console(name = "harness:report", description = "Prints, warns and may fail")]
struct Report {
    /// End with an error after the first lines were printed.
    #[arg(long)]
    fail: bool,
}

#[async_trait]
impl TypedCommand for Report {
    async fn run(self) -> Result<(), FrameworkError> {
        console::line("3 rows checked");
        console::error_line("1 row skipped");
        if self.fail {
            return Err(FrameworkError::internal("the report could not be written"));
        }
        console::line("done");
        Ok(())
    }
}

#[derive(Parser, Command, Debug)]
#[console(name = "harness:purge", description = "Asks before it deletes")]
struct Purge {}

#[async_trait]
impl TypedCommand for Purge {
    async fn run(self) -> Result<(), FrameworkError> {
        let table = console::ask("Which table?")?;
        if !console::confirm(&format!("Delete every row of {table}?"), false)? {
            console::line("nothing deleted");
            return Ok(());
        }
        console::line(format!("deleted every row of {table}"));
        Ok(())
    }
}

#[cfg(feature = "testing")]
struct HarnessSeeder;

#[cfg(feature = "testing")]
#[async_trait]
impl Seeder for HarnessSeeder {
    fn name() -> &'static str {
        "HarnessSeeder"
    }
    async fn run() -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
async fn the_run_has_what_the_command_printed_on_each_stream() {
    let run = console::test(["harness:report"]).run().await;

    run.assert_successful();
    assert_eq!(run.exit_code(), 0);
    assert_eq!(run.output(), "3 rows checked\ndone\n");
    assert_eq!(run.errors(), "1 row skipped\n");
    assert!(run.error().is_none());
}

#[tokio::test]
async fn a_failed_command_ends_with_exit_code_1_and_its_error_on_the_error_stream() {
    let run = console::test(["harness:report", "--fail"]).run().await;

    run.assert_failed();
    assert_eq!(run.exit_code(), 1);
    assert_eq!(
        run.output(),
        "3 rows checked\n",
        "what was printed before the failure is kept"
    );
    assert_eq!(
        run.errors(),
        "1 row skipped\nerror: the report could not be written\n",
        "the console prints the error of a failed command, as the binary does"
    );
    assert_eq!(
        run.error().map(FrameworkError::message),
        Some("the report could not be written")
    );
}

#[tokio::test]
async fn a_question_is_answered_with_what_the_test_prepared() {
    let run = console::test(["harness:purge"])
        .expects_question("Which table?", "sessions")
        .expects_question("Delete every row of sessions?", "yes")
        .run()
        .await;

    run.assert_successful()
        .assert_every_question_was_asked()
        .assert_output_contains("deleted every row of sessions");
    assert_eq!(
        run.output(),
        "Which table?\nDelete every row of sessions?\ndeleted every row of sessions\n",
        "the questions are a part of the output"
    );
}

#[tokio::test]
async fn the_answer_decides_what_the_command_does() {
    let run = console::test(["harness:purge"])
        .expects_question("Which table?", "sessions")
        .expects_question("Delete every row of sessions?", "no")
        .run()
        .await;

    run.assert_successful()
        .assert_output_contains("nothing deleted");
    assert!(!run.output().contains("deleted every row"));
}

#[tokio::test]
async fn a_question_the_test_did_not_prepare_fails_the_command() {
    // Without this a command that asks would wait on the standard input
    // of the test runner.
    let run = console::test(["harness:purge"]).run().await;

    run.assert_failed().assert_errors_contain("Which table?");
    assert!(
        !run.output().contains("deleted"),
        "a command with no answer must not go on: {}",
        run.output()
    );
}

#[tokio::test]
async fn an_answer_is_not_given_to_another_question() {
    let run = console::test(["harness:purge"])
        .expects_question("Which table?", "sessions")
        .expects_question("Delete every row of users?", "yes")
        .run()
        .await;

    run.assert_failed()
        .assert_errors_contain("Delete every row of sessions?");
    assert!(
        !run.output().contains("deleted every row"),
        "the yes was written for `users`, and the command asked about `sessions`"
    );
    assert_eq!(run.unasked_questions(), ["Delete every row of users?"]);
}

#[tokio::test]
async fn a_question_the_command_never_asks_is_reported() {
    let run = console::test(["harness:report"])
        .expects_question("Are you sure?", "yes")
        .run()
        .await;

    run.assert_successful();
    assert_eq!(run.unasked_questions(), ["Are you sure?"]);
}

#[tokio::test]
async fn the_help_of_the_console_is_read_from_the_output() {
    let run = console::test(["--help"]).run().await;

    run.assert_successful()
        .assert_output_contains("harness:report")
        .assert_output_contains("Prints, warns and may fail");
    assert_eq!(run.errors(), "");
}

#[tokio::test]
async fn the_help_of_one_command_is_read_from_the_output() {
    let run = console::test(["harness:report", "--help"]).run().await;

    run.assert_successful()
        .assert_output_contains("--fail")
        .assert_output_contains("End with an error after the first lines were printed");
}

#[tokio::test]
async fn an_argument_the_command_does_not_take_fails_on_the_error_stream() {
    let run = console::test(["harness:report", "--nope"]).run().await;

    run.assert_failed().assert_errors_contain("--nope");
    assert_eq!(run.exit_code(), 1);
    assert_eq!(run.output(), "", "the command did not run");
}

#[tokio::test]
async fn a_command_that_does_not_exist_fails_on_the_error_stream() {
    let run = console::test(["harness:missing"]).run().await;

    run.assert_failed().assert_errors_contain("harness:missing");
    assert_eq!(run.output(), "");
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial]
async fn a_builtin_command_is_read_the_same_way() {
    seed::clear();
    seed::register::<HarnessSeeder>();

    let run = console::test(["db:seed", "--class=HarnessSeeder"])
        .run()
        .await;
    seed::clear();

    run.assert_successful();
    let lines: Vec<&str> = run.output().lines().collect();
    assert_eq!(lines.len(), 3, "RUNNING, DONE and an empty line: {lines:?}");
    assert!(
        lines[0].starts_with("  HarnessSeeder ") && lines[0].ends_with(" RUNNING"),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("  HarnessSeeder ") && lines[1].ends_with(" ms DONE"),
        "{lines:?}"
    );
    assert_eq!(lines[2], "");
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial]
async fn a_warning_of_a_builtin_command_is_on_the_error_stream() {
    seed::clear();

    let run = console::test(["db:seed"]).run().await;

    run.assert_successful()
        .assert_errors_contain("no seeders registered");
    assert_eq!(run.output(), "");
}

/// Every source file of the framework that holds the body of a console
/// command.
fn command_sources() -> Vec<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![
        src.join("console/mod.rs"),
        src.join("render_cache/console.rs"),
        src.join("live/tooling.rs"),
    ];
    for dir in ["console/builtins", "eloquent/console"] {
        let dir = src.join(dir);
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

/// A command of the framework prints through the console, or a test of
/// an application cannot read what it printed.
///
/// `println!` compiles and looks right in a terminal, so nothing else
/// says when one comes back.
#[test]
fn no_command_of_the_framework_prints_past_the_console() {
    let files = command_sources();
    assert!(
        files.len() >= 5,
        "the list of command sources is short, the walk is broken: {files:?}"
    );

    let mut found = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        for (index, line) in source.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let prints = ["println!(", "eprintln!(", "print!(", "eprint!("]
                .iter()
                .any(|call| code.contains(call));
            if prints {
                found.push(format!("{}:{}: {}", file.display(), index + 1, code));
            }
        }
    }

    assert!(
        found.is_empty(),
        "these lines print to the standard streams of the process; use \
         `console::line` or `console::error_line`:\n{}",
        found.join("\n")
    );
}
