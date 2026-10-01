//! Where a console command writes, and where its answers come from.
//!
//! A command that prints with `println!` writes to the standard output of
//! the process, which a test cannot read, and a command that reads the
//! standard input cannot be answered by a test. The functions here use the
//! standard streams when the command runs in the console binary, and a
//! buffer and a list of prepared answers when it runs under
//! [`test`](super::testing::test).

use crate::error::FrameworkError;
use std::collections::VecDeque;
use std::fmt::Display;
use std::future::Future;
use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

tokio::task_local! {
    /// What the running test collects. Absent in the console binary.
    static CAPTURE: Arc<Capture>;
}

/// The output of one test run, and the answers it still holds.
#[derive(Default)]
pub(super) struct Captured {
    pub(super) output: String,
    pub(super) errors: String,
    /// The questions the test expects, in order, each with its answer.
    pub(super) answers: VecDeque<(String, String)>,
}

#[derive(Default)]
pub(super) struct Capture {
    state: Mutex<Captured>,
}

impl Capture {
    pub(super) fn with_answers(answers: VecDeque<(String, String)>) -> Self {
        Self {
            state: Mutex::new(Captured {
                answers,
                ..Captured::default()
            }),
        }
    }

    /// The state, whatever happened to the last holder of the lock. A
    /// command that panicked while it printed must not hide what was
    /// printed before it from the test.
    fn lock(&self) -> MutexGuard<'_, Captured> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Take what was collected, and leave the capture empty.
    pub(super) fn take(&self) -> Captured {
        std::mem::take(&mut *self.lock())
    }

    /// The prepared answer to `question`. The questions have to come in
    /// the order the test gave them: an answer that went to another
    /// question than the one it was written for would let a test pass
    /// for the wrong reason.
    fn answer(&self, question: &str) -> Result<String, FrameworkError> {
        let mut captured = self.lock();
        let next = captured
            .answers
            .front()
            .map(|(expected, _)| expected.clone());
        match next {
            Some(expected) if expected == question => {
                captured.output.push_str(question);
                captured.output.push('\n');
                Ok(captured
                    .answers
                    .pop_front()
                    .map(|(_, answer)| answer)
                    .unwrap_or_default())
            }
            Some(expected) => Err(FrameworkError::internal(format!(
                "console test: the command asked `{question}`, and the question the test \
                 expects next is `{expected}`"
            ))),
            None => Err(FrameworkError::internal(format!(
                "console test: the command asked `{question}`, and the test has no answer \
                 left; add `.expects_question(\"{question}\", ...)`"
            ))),
        }
    }
}

fn capture() -> Option<Arc<Capture>> {
    CAPTURE.try_with(Arc::clone).ok()
}

/// Whether the output is collected for a test.
pub(super) fn is_captured() -> bool {
    capture().is_some()
}

/// Run `future` with its console output collected in `capture`.
pub(super) async fn collect_into<F: Future>(capture: Arc<Capture>, future: F) -> F::Output {
    CAPTURE.scope(capture, future).await
}

/// Write `text` as it is to the standard output, or to the test.
pub(super) fn write_output(text: &str) {
    match capture() {
        Some(capture) => capture.lock().output.push_str(text),
        None => {
            // A write to a pipe the reader has closed (`console list |
            // head`) fails, and `println!` panics on it. There is no
            // stream left to report the failure on, and a command must
            // not end in a panic because its reader went away.
            let _ = std::io::stdout().lock().write_all(text.as_bytes());
        }
    }
}

/// Write `text` as it is to the standard error, or to the test.
pub(super) fn write_errors(text: &str) {
    match capture() {
        Some(capture) => capture.lock().errors.push_str(text),
        None => {
            // As in `write_output`: nowhere is left to report it.
            let _ = std::io::stderr().lock().write_all(text.as_bytes());
        }
    }
}

/// Print one line for the person who ran the command.
///
/// Use this where you would use `println!`. It writes to the standard
/// output when the command runs in the console binary, and a test that
/// runs the command with [`test`](super::testing::test) reads the line
/// from [`ConsoleRun::output`](super::testing::ConsoleRun::output). What
/// `println!` prints, the test cannot see.
///
/// ```rust
/// suprnova::console::line(format!("pruned {} rows", 3));
/// ```
pub fn line(text: impl Display) {
    write_output(&format!("{text}\n"));
}

/// Print one line on the standard error: a warning, or a note that must
/// not end up in output that is piped into another program. A test reads
/// it from [`ConsoleRun::errors`](super::testing::ConsoleRun::errors).
///
/// A command that fails returns the error. The console prints the error
/// of a failed command itself, and printing it here as well shows it
/// twice.
pub fn error_line(text: impl Display) {
    write_errors(&format!("{text}\n"));
}

/// Ask a question and return the line that was typed, without its line
/// ending.
///
/// In a test the answer is the one the test prepared with
/// [`expects_question`](super::testing::ConsoleTest::expects_question).
///
/// This waits on the standard input and blocks the thread while it does.
/// That is right for a console command, which has nothing else to do
/// until it is answered, and wrong inside a server.
///
/// # Errors
///
/// When the input ends before a line was read, which is what a command
/// gets when it is run with no terminal and nothing piped in: a command
/// that went on with an empty answer would act on an answer nobody gave.
/// In a test, when the test did not expect this question next.
pub fn ask(question: &str) -> Result<String, FrameworkError> {
    ask_with_hint(question, "")
}

/// Ask a question that is answered with yes or no.
///
/// `y` and `yes` are yes, `n` and `no` are no, in any case. An empty
/// line is `default`. The hint behind the question, `[Y/n]` or `[y/N]`,
/// shows which one the default is. A test answers the question alone,
/// without the hint: `.expects_question("Delete every row?", "yes")`.
///
/// # Errors
///
/// When the answer is none of those. A command asks before it does
/// something that cannot be undone, and reading an unclear answer as
/// either yes or no would be a guess. Also every error of [`ask`].
pub fn confirm(question: &str, default: bool) -> Result<bool, FrameworkError> {
    let hint = if default { " [Y/n]" } else { " [y/N]" };
    let answer = ask_with_hint(question, hint)?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => Err(FrameworkError::bad_request(format!(
            "`{other}` is no answer to `{question}`: answer yes or no"
        ))),
    }
}

fn ask_with_hint(question: &str, hint: &str) -> Result<String, FrameworkError> {
    if let Some(capture) = capture() {
        return capture.answer(question);
    }

    {
        let mut out = std::io::stdout().lock();
        write!(out, "{question}{hint} ")
            .and_then(|()| out.flush())
            .map_err(|e| {
                FrameworkError::internal(format!("console: cannot print `{question}`: {e}"))
            })?;
    }
    let mut answer = String::new();
    let read = std::io::stdin().read_line(&mut answer).map_err(|e| {
        FrameworkError::internal(format!(
            "console: cannot read the answer to `{question}`: {e}"
        ))
    })?;
    if read == 0 {
        return Err(FrameworkError::bad_request(format!(
            "`{question}` was not answered: the input ended. Run the command in a terminal, \
             or pipe the answer in"
        )));
    }
    Ok(answer.trim_end_matches(['\r', '\n']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collected<F: Future>(answers: &[(&str, &str)], future: F) -> (F::Output, Captured) {
        let capture = Arc::new(Capture::with_answers(
            answers
                .iter()
                .map(|(q, a)| ((*q).to_owned(), (*a).to_owned()))
                .collect(),
        ));
        let output = collect_into(capture.clone(), future).await;
        (output, capture.take())
    }

    #[tokio::test]
    async fn lines_go_to_the_stream_they_were_written_for() {
        let ((), captured) = collected(&[], async {
            line("one");
            error_line("careful");
            line(2);
        })
        .await;

        assert_eq!(captured.output, "one\n2\n");
        assert_eq!(captured.errors, "careful\n");
    }

    #[tokio::test]
    async fn a_question_gets_the_answer_the_test_prepared() {
        let (answers, captured) = collected(&[("Name?", "Ada"), ("Town?", "London")], async {
            (ask("Name?"), ask("Town?"))
        })
        .await;

        assert_eq!(answers.0.expect("the first answer"), "Ada");
        assert_eq!(answers.1.expect("the second answer"), "London");
        assert_eq!(
            captured.output, "Name?\nTown?\n",
            "the question is a part of what the command printed"
        );
        assert!(captured.answers.is_empty());
    }

    #[tokio::test]
    async fn a_question_out_of_order_is_an_error_and_uses_no_answer() {
        let (answer, captured) =
            collected(&[("Name?", "Ada")], async { ask("Delete every row?") }).await;

        let error = answer.expect_err("the test expected another question");
        assert!(
            error.message().contains("Delete every row?") && error.message().contains("Name?"),
            "the error must name both questions: {}",
            error.message()
        );
        assert_eq!(captured.answers.len(), 1, "the answer is still there");
    }

    #[tokio::test]
    async fn a_question_the_test_has_no_answer_for_is_an_error() {
        let (answer, _) = collected(&[], async { ask("Name?") }).await;
        assert!(answer.is_err());
    }

    #[tokio::test]
    async fn confirm_reads_yes_no_and_the_default() {
        for (typed, default, expected) in [
            ("yes", false, true),
            ("Y", false, true),
            ("no", true, false),
            ("N", true, false),
            ("", true, true),
            ("", false, false),
            ("  yes  ", false, true),
        ] {
            let (answer, _) =
                collected(&[("Go on?", typed)], async { confirm("Go on?", default) }).await;
            assert_eq!(
                answer.expect("an answer"),
                expected,
                "`{typed}` with default {default}"
            );
        }
    }

    #[tokio::test]
    async fn confirm_refuses_an_answer_that_is_neither() {
        let (answer, _) =
            collected(&[("Go on?", "maybe")], async { confirm("Go on?", true) }).await;
        let error = answer.expect_err("`maybe` is no answer");
        assert!(error.message().contains("maybe"), "{}", error.message());
    }

    #[tokio::test]
    async fn outside_a_test_nothing_is_collected() {
        assert!(!is_captured());
        let ((), _) = collected(&[], async { assert!(is_captured()) }).await;
        assert!(!is_captured());
    }
}
