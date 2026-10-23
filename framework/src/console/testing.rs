//! Run a console command in a test and read what it printed.
//!
//! [`dispatch_argv`](super::dispatch_argv) returns a `Result` and nothing
//! else. The output of the command went to the standard streams of the
//! test process, so a test could tell that a command ran and not what it
//! said, and a command that asks a question waited on the standard input
//! of the test runner.

use super::io::{Capture, Expected, Written, collect_into};
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
/// Expectations can also be stated up front, as Laravel's `artisan` test
/// states them, and [`ConsoleRun::assert_successful`] checks every one:
///
/// ```rust,no_run
/// # async fn example() {
/// suprnova::console::test(["users:purge", "--days", "30"])
///     .expects_confirmation("Delete 12 users?", true)
///     .expects_output("deleted 12 users")
///     .doesnt_expect_output_to_contain("skipped")
///     .run()
///     .await
///     .assert_successful();
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
/// What the command writes with [`line`](super::line),
/// [`error_line`](super::error_line), their `_at` forms,
/// [`error`](super::error), [`warn`](super::warn), [`info`](super::info)
/// and a [`Progress`](super::Progress) bar, the questions its prompts ask
/// (never a [`secret`](super::secret) answer), and what the console itself
/// prints: help, the version, parse errors, and the error a command
/// returned. All of it is plain text, with no terminal styles. What a
/// command prints with `println!` is not collected, and neither is what a
/// task that the command spawned prints: the collection belongs to the
/// task the command runs on.
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
        lines: Vec::new(),
        contains: Vec::new(),
        lacks: Vec::new(),
    }
}

/// A run of the console that has not started. Built by [`test()`].
#[must_use = "a prepared run does nothing until `.run().await`"]
pub struct ConsoleTest {
    argv: Vec<String>,
    answers: VecDeque<Expected>,
    /// The lines the command must print, in this order.
    lines: Vec<String>,
    /// The texts one write of the command must contain.
    contains: Vec<String>,
    /// The texts no write of the command may contain.
    lacks: Vec<String>,
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
    ///
    /// A question the test did not expect at that point fails every
    /// assertion on the run, naming it, even when the command goes on past
    /// the error; a prepared question the command never asks fails
    /// [`ConsoleRun::assert_successful`].
    pub fn expects_question(
        mut self,
        question: impl Into<String>,
        answer: impl Into<String>,
    ) -> Self {
        self.answers
            .push_back(Expected::new(question.into(), answer.into(), None));
        self
    }

    /// Answer the yes-or-no `question` of a [`confirm`](super::confirm)
    /// with yes when `answer` is `true` and no when it is `false`, as
    /// Laravel's `expectsConfirmation` does.
    ///
    /// The question is the text without the `[y/N]` hint, and it takes its
    /// place in the order of the questions as
    /// [`expects_question`](Self::expects_question) does.
    pub fn expects_confirmation(self, question: impl Into<String>, answer: bool) -> Self {
        self.expects_question(question, if answer { "yes" } else { "no" })
    }

    /// Expect the command to print `line`, exactly, as one whole line.
    ///
    /// Several calls expect their lines in the order of the calls; lines
    /// between them are allowed. A line is looked for on the standard
    /// output and the standard error alike, since a Laravel command writes
    /// its error lines to its one output, and a question counts as the
    /// line it adds to the output. A write of several lines gives each of
    /// them, and a line is never joined from two writes. A line not
    /// printed, or printed before the line expected ahead of it, fails
    /// [`ConsoleRun::assert_successful`] and is listed by
    /// [`ConsoleRun::unmet_expectations`].
    pub fn expects_output(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// Expect one write of the command, on the standard output or the
    /// standard error, to contain `text`.
    ///
    /// A write is one [`line`](super::line), one question, or one message
    /// of the console, so a match never spans two writes: `bo` and `om`
    /// printed apart do not contain `boom`. Unmet, it fails
    /// [`ConsoleRun::assert_successful`] and is listed by
    /// [`ConsoleRun::unmet_expectations`].
    pub fn expects_output_to_contain(mut self, text: impl Into<String>) -> Self {
        self.contains.push(text.into());
        self
    }

    /// Expect no write of the command, on either stream, to contain
    /// `text`. A write that does fails [`ConsoleRun::assert_successful`];
    /// a match never spans two writes, as for
    /// [`expects_output_to_contain`](Self::expects_output_to_contain).
    pub fn doesnt_expect_output_to_contain(mut self, text: impl Into<String>) -> Self {
        self.lacks.push(text.into());
        self
    }

    /// Answer the menu `question` with `answer`, and expect the menu to
    /// offer exactly `options`, in that order.
    ///
    /// For [`select`](super::select), [`select_keyed`](super::select_keyed)
    /// and [`multiselect`](super::multiselect). The answer is typed as a
    /// person types it: an option's label, and for a multiselect the labels
    /// separated by commas, `"Member,Owner"`. A menu that offers other
    /// options fails the command, and so does a prompt that is no menu, so
    /// the test notices when the choices change. Use
    /// [`expects_question`](Self::expects_question) to answer a menu without
    /// checking its options.
    pub fn expects_choice<I, S>(
        mut self,
        question: impl Into<String>,
        answer: impl Into<String>,
        options: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.answers.push_back(Expected::new(
            question.into(),
            answer.into(),
            Some(options.into_iter().map(Into::into).collect()),
        ));
        self
    }

    /// Run the command, collect what it printed, and check the
    /// expectations against it.
    pub async fn run(self) -> ConsoleRun {
        let ConsoleTest {
            argv,
            answers,
            lines,
            contains,
            lacks,
        } = self;
        let capture = Arc::new(Capture::with_answers(answers));
        let result = collect_into(capture.clone(), super::dispatch_argv(argv)).await;
        let captured = capture.take();

        let mut unmet: Vec<String> = captured
            .answers
            .iter()
            .map(|expected| match &expected.refusal {
                Some(refusal) => refusal.clone(),
                None => format!("the question `{}` was not asked", expected.question),
            })
            .collect();
        unmet.extend(missing_lines(&captured.writes, &lines));
        unmet.extend(
            contains
                .iter()
                .filter(|text| {
                    !captured
                        .writes
                        .iter()
                        .any(|w| w.text.contains(text.as_str()))
                })
                .map(|text| format!("no write of the output or the errors contains `{text}`")),
        );
        unmet.extend(lacks.iter().filter_map(|text| {
            captured
                .writes
                .iter()
                .find(|w| w.text.contains(text.as_str()))
                .map(|w| {
                    format!(
                        "the command printed `{text}`, which the test does not expect, on {} \
                         in `{}`",
                        w.stream.name(),
                        w.text.trim_end_matches('\n')
                    )
                })
        }));

        ConsoleRun {
            output: captured.output,
            errors: captured.errors,
            unasked: captured
                .answers
                .into_iter()
                .map(|expected| expected.question)
                .collect(),
            unexpected: captured.unexpected,
            unmet,
            result,
        }
    }
}

/// The lines of `expected` that `writes` do not hold in that order, each
/// as the sentence a failed assertion shows.
///
/// Each line is looked for after the line found before it, so lines in
/// between are allowed and a line printed too early is not found. Every
/// write is split into its lines on its own, so a line is never joined from
/// the end of one write and the start of the next.
fn missing_lines(writes: &[Written], expected: &[String]) -> Vec<String> {
    let printed: Vec<&str> = writes.iter().flat_map(|w| w.text.lines()).collect();
    let mut from = 0;
    let mut found_before: Option<&str> = None;
    let mut missing = Vec::new();
    for line in expected {
        match printed[from..].iter().position(|p| p == line) {
            Some(offset) => {
                from += offset + 1;
                found_before = Some(line);
            }
            None => missing.push(match found_before {
                Some(before) if printed.contains(&line.as_str()) => {
                    format!("the line `{line}` was not printed after `{before}`")
                }
                _ => format!("the line `{line}` was not printed"),
            }),
        }
    }
    missing
}

/// What a run of the console printed and how it ended.
#[derive(Debug)]
pub struct ConsoleRun {
    output: String,
    errors: String,
    unasked: Vec<String>,
    unexpected: Vec<String>,
    unmet: Vec<String>,
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
    /// when the command succeeded, the code a command chose with
    /// [`FrameworkError::exit`], and `1` when it failed otherwise or the
    /// arguments did not parse. Help and the version end with `0`.
    pub fn exit_code(&self) -> u8 {
        match &self.result {
            Ok(()) => 0,
            Err(error) => error.exit_code(),
        }
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

    /// The questions the command asked that the test did not expect at
    /// that point, in the order they were asked: the test had no answer
    /// left, or it expected another question next.
    ///
    /// Each one fails every assertion on how the run ended, as Laravel
    /// fails the test at an unexpected question, so a test cannot pass
    /// because a command got an answer written for another question.
    pub fn unexpected_questions(&self) -> &[String] {
        &self.unexpected
    }

    /// Every expectation of the test the command did not meet, one
    /// sentence each: a prepared question it did not ask or whose answer
    /// it refused, an expected line it did not print or printed out of
    /// order, an expected text no write contains, and a text a write
    /// contains that the test does not expect. In that order, and in the
    /// order of the calls within each kind.
    pub fn unmet_expectations(&self) -> &[String] {
        &self.unmet
    }

    /// Assert that the run ended with exit code `0`, that the command
    /// asked no question the test did not expect, and that it met every
    /// expectation of the test.
    ///
    /// The failure lists every problem at once, with both streams.
    #[track_caller]
    pub fn assert_successful(&self) -> &Self {
        let mut problems = Vec::new();
        if self.exit_code() != 0 {
            problems.push(format!(
                "the command failed with exit code {}",
                self.exit_code()
            ));
        }
        problems.extend(self.unexpected_problems());
        problems.extend(self.unmet_problems());
        assert!(
            problems.is_empty(),
            "{}\n{}",
            problems.join("\n"),
            self.streams()
        );
        self
    }

    /// Assert that the run ended with a failure: an exit code other than
    /// `0`.
    ///
    /// A question the test did not expect fails it too, naming the
    /// question: the run failed for a reason the test did not state. The
    /// other expectations are not checked here, since a test of a refusal,
    /// such as a menu that offers other options, keeps the answer the
    /// command refused; [`assert_not_exit_code(0)`](Self::assert_not_exit_code)
    /// checks them.
    #[track_caller]
    pub fn assert_failed(&self) -> &Self {
        self.assert_no_unexpected_question();
        assert!(
            self.exit_code() != 0,
            "the command succeeded and was expected to fail.\n--- output ---\n{}",
            self.output
        );
        self
    }

    /// Assert that the run ended with exit code `code`, as Laravel's
    /// `assertExitCode` does, with the questions and expectations checked
    /// as [`assert_successful`](Self::assert_successful) checks them.
    ///
    /// A command ends with a code of its own through
    /// [`FrameworkError::exit`].
    #[track_caller]
    pub fn assert_exit_code(&self, code: u8) -> &Self {
        let mut problems = Vec::new();
        if self.exit_code() != code {
            problems.push(format!(
                "the command ended with exit code {}, and the test expects exit code {code}",
                self.exit_code()
            ));
        }
        problems.extend(self.unexpected_problems());
        problems.extend(self.unmet_problems());
        assert!(
            problems.is_empty(),
            "{}\n{}",
            problems.join("\n"),
            self.streams()
        );
        self
    }

    /// Assert that the run ended with any exit code but `code`, as
    /// Laravel's `assertNotExitCode` does, with the questions and
    /// expectations checked as
    /// [`assert_successful`](Self::assert_successful) checks them.
    #[track_caller]
    pub fn assert_not_exit_code(&self, code: u8) -> &Self {
        let mut problems = Vec::new();
        if self.exit_code() == code {
            problems.push(format!(
                "the command ended with exit code {code}, which the test does not expect"
            ));
        }
        problems.extend(self.unexpected_problems());
        problems.extend(self.unmet_problems());
        assert!(
            problems.is_empty(),
            "{}\n{}",
            problems.join("\n"),
            self.streams()
        );
        self
    }

    /// Assert that the standard output contains `text`.
    ///
    /// A question the test did not expect fails it first, naming the
    /// question, as it fails every other assertion on the run: a text
    /// that the output holds does not show that the run went as the test
    /// stated.
    #[track_caller]
    pub fn assert_output_contains(&self, text: &str) -> &Self {
        self.assert_no_unexpected_question();
        assert!(
            self.output.contains(text),
            "the output does not contain `{text}`.\n--- output ---\n{}",
            self.output
        );
        self
    }

    /// Assert that the standard error contains `text`.
    ///
    /// A question the test did not expect fails it first, naming the
    /// question, as it fails every other assertion on the run.
    #[track_caller]
    pub fn assert_errors_contain(&self, text: &str) -> &Self {
        self.assert_no_unexpected_question();
        assert!(
            self.errors.contains(text),
            "the errors do not contain `{text}`.\n--- errors ---\n{}",
            self.errors
        );
        self
    }

    /// Assert that the command asked every question the test prepared an
    /// answer for.
    ///
    /// A question the test did not expect fails it first, naming the
    /// question, as it fails every other assertion on the run.
    #[track_caller]
    pub fn assert_every_question_was_asked(&self) -> &Self {
        self.assert_no_unexpected_question();
        assert!(
            self.unasked.is_empty(),
            "the command did not ask: {:?}",
            self.unasked
        );
        self
    }

    /// Fails at the questions the test did not expect, naming each one
    /// with both streams. The assertions that check one stream, the list
    /// of asked questions or the failure call it first, so none of them
    /// can pass on a run the test did not state. The exit-code assertions
    /// fold the same message into their own list of problems.
    #[track_caller]
    fn assert_no_unexpected_question(&self) {
        let unexpected = self.unexpected_problems();
        assert!(
            unexpected.is_empty(),
            "{}\n{}",
            unexpected.join("\n"),
            self.streams()
        );
    }

    /// One line for each question the test did not expect.
    fn unexpected_problems(&self) -> Vec<String> {
        self.unexpected
            .iter()
            .map(|question| {
                format!(
                    "unexpected question: the command asked `{question}`, which the test \
                     did not expect at that point"
                )
            })
            .collect()
    }

    /// One line for each expectation the command did not meet.
    fn unmet_problems(&self) -> Vec<String> {
        self.unmet
            .iter()
            .map(|unmet| format!("unmet expectation: {unmet}"))
            .collect()
    }

    /// Both streams, for the message of a failed assertion.
    fn streams(&self) -> String {
        format!(
            "--- output ---\n{}\n--- errors ---\n{}",
            self.output, self.errors
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::io::Stream;
    use super::*;

    fn out(text: &str) -> Written {
        Written {
            stream: Stream::Output,
            text: text.to_owned(),
        }
    }

    fn err(text: &str) -> Written {
        Written {
            stream: Stream::Errors,
            text: text.to_owned(),
        }
    }

    fn lines(expected: &[&str]) -> Vec<String> {
        expected.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn lines_are_found_in_order_with_lines_between() {
        let writes = [out("a\n"), err("x\n"), out("b\n")];
        assert!(missing_lines(&writes, &lines(&["a", "b"])).is_empty());
        assert!(missing_lines(&writes, &lines(&["x"])).is_empty());
    }

    #[test]
    fn a_line_printed_before_the_one_it_must_follow_is_missing() {
        let writes = [out("b\n"), out("a\n")];
        assert_eq!(
            missing_lines(&writes, &lines(&["a", "b"])),
            ["the line `b` was not printed after `a`"]
        );
    }

    #[test]
    fn a_line_is_matched_whole_and_never_joined_from_two_writes() {
        let writes = [out("bo"), out("om\n"), out("hello\n")];
        assert_eq!(
            missing_lines(&writes, &lines(&["boom", "hel"])),
            [
                "the line `boom` was not printed",
                "the line `hel` was not printed"
            ]
        );
        assert!(missing_lines(&writes, &lines(&["bo", "om", "hello"])).is_empty());
    }

    #[test]
    fn a_missing_line_does_not_hide_the_lines_after_it() {
        let writes = [out("a\n"), out("b\n")];
        assert_eq!(
            missing_lines(&writes, &lines(&["a", "never", "b"])),
            ["the line `never` was not printed"]
        );
    }
}
