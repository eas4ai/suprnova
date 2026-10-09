//! Run a console command in a test and read what it printed.
//!
//! [`dispatch_argv`](super::dispatch_argv) returns a `Result` and nothing
//! else. The output of the command went to the standard streams of the
//! test process, so a test could tell that a command ran and not what it
//! said, and a command that asks a question waited on the standard input
//! of the test runner.

use super::io::{Capture, collect_into};
use crate::error::FrameworkError;
use std::collections::VecDeque;
use std::sync::Arc;

/// Prepare a run of the console with `argv`, the way it is typed behind
/// the name of the binary:
///
/// ```rust,no_run
/// # async fn example() {
/// let run = suprnova::console::test(["users:purge", "--days", "30"])
///     .expects_question("Delete 12 users?", "yes")
///     .run()
///     .await;
///
/// run.assert_successful().assert_every_question_was_asked();
/// assert!(run.output().contains("deleted 12 users"));
/// # }
/// ```
///
/// The run goes through the same dispatcher as the console binary: clap
/// parses `argv`, the command that matches runs, and the error of a
/// failed command is printed on the standard error. So `["--help"]` and
/// an argument the command does not take can be tested as well.
///
/// # What is collected
///
/// What the command writes with [`line`](super::line) and
/// [`error_line`](super::error_line), what it asks with
/// [`ask`](super::ask) and [`confirm`](super::confirm), and what the
/// console itself prints: help, the version, parse errors, and the error
/// a command returned. What a command prints with `println!` is not
/// collected, and neither is what a task that the command spawned
/// prints: the collection belongs to the task the command runs on.
pub fn test<I, S>(argv: I) -> ConsoleTest
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ConsoleTest {
        argv: std::iter::once("console".to_owned())
            .chain(argv.into_iter().map(Into::into))
            .collect(),
        answers: VecDeque::new(),
    }
}

/// A run of the console that has not started. Built by [`test()`].
#[must_use = "a prepared run does nothing until `.run().await`"]
pub struct ConsoleTest {
    argv: Vec<String>,
    answers: VecDeque<(String, String)>,
}

impl ConsoleTest {
    /// Answer `question` with `answer` when the command asks it.
    ///
    /// The command has to ask its questions in the order they are given
    /// here. A question that comes out of order, or one that was not
    /// given, makes [`ask`](super::ask) return an error, so the command
    /// fails and does not go on with an answer that was written for
    /// another question. For [`confirm`](super::confirm) the question is
    /// the text without the `[y/N]` hint.
    pub fn expects_question(
        mut self,
        question: impl Into<String>,
        answer: impl Into<String>,
    ) -> Self {
        self.answers.push_back((question.into(), answer.into()));
        self
    }

    /// Run the command and collect what it printed.
    pub async fn run(self) -> ConsoleRun {
        let capture = Arc::new(Capture::with_answers(self.answers));
        let result = collect_into(capture.clone(), super::dispatch_argv(self.argv)).await;
        let captured = capture.take();
        ConsoleRun {
            output: captured.output,
            errors: captured.errors,
            unasked: captured
                .answers
                .into_iter()
                .map(|(question, _)| question)
                .collect(),
            result,
        }
    }
}

/// What a run of the console printed and how it ended.
#[derive(Debug)]
pub struct ConsoleRun {
    output: String,
    errors: String,
    unasked: Vec<String>,
    result: Result<(), FrameworkError>,
}

impl ConsoleRun {
    /// What was written to the standard output, questions included.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// What was written to the standard error. The error of a command
    /// that failed is in here, as `error: <message>`.
    pub fn errors(&self) -> &str {
        &self.errors
    }

    /// The exit code the console binary ends with after this run: `0`
    /// when the command succeeded, and `1` when it failed or the
    /// arguments did not parse. Help and the version end with `0`.
    pub fn exit_code(&self) -> u8 {
        u8::from(self.result.is_err())
    }

    /// The error the run ended with, when it failed.
    pub fn error(&self) -> Option<&FrameworkError> {
        self.result.as_ref().err()
    }

    /// The questions the test prepared an answer for and the command did
    /// not ask, in the order they were given.
    pub fn unasked_questions(&self) -> &[String] {
        &self.unasked
    }

    /// Assert that the run ended with exit code `0`.
    #[track_caller]
    pub fn assert_successful(&self) -> &Self {
        assert!(
            self.result.is_ok(),
            "the command failed.\n--- output ---\n{}\n--- errors ---\n{}",
            self.output,
            self.errors
        );
        self
    }

    /// Assert that the run ended with a failure.
    #[track_caller]
    pub fn assert_failed(&self) -> &Self {
        assert!(
            self.result.is_err(),
            "the command succeeded and was expected to fail.\n--- output ---\n{}",
            self.output
        );
        self
    }

    /// Assert that the standard output contains `text`.
    #[track_caller]
    pub fn assert_output_contains(&self, text: &str) -> &Self {
        assert!(
            self.output.contains(text),
            "the output does not contain `{text}`.\n--- output ---\n{}",
            self.output
        );
        self
    }

    /// Assert that the standard error contains `text`.
    #[track_caller]
    pub fn assert_errors_contain(&self, text: &str) -> &Self {
        assert!(
            self.errors.contains(text),
            "the errors do not contain `{text}`.\n--- errors ---\n{}",
            self.errors
        );
        self
    }

    /// Assert that the command asked every question the test prepared an
    /// answer for.
    #[track_caller]
    pub fn assert_every_question_was_asked(&self) -> &Self {
        assert!(
            self.unasked.is_empty(),
            "the command did not ask: {:?}",
            self.unasked
        );
        self
    }
}
