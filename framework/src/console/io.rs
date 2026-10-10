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
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

tokio::task_local! {
    /// What the running test collects. Absent in the console binary.
    static CAPTURE: Arc<Capture>;
}

/// One question a test expects, with the answer it gives.
pub(super) struct Expected {
    pub(super) question: String,
    pub(super) answer: String,
    /// The options a menu must offer, for an answer prepared with
    /// [`expects_choice`](super::testing::ConsoleTest::expects_choice).
    /// `None` answers any prompt, a menu included.
    pub(super) options: Option<Vec<String>>,
}

/// The output of one test run, and the answers it still holds.
#[derive(Default)]
pub(super) struct Captured {
    pub(super) output: String,
    pub(super) errors: String,
    /// The questions the test expects, in order, each with its answer.
    pub(super) answers: VecDeque<Expected>,
    /// The level the run's `-q` and `-v` flags asked for.
    pub(super) verbosity: Verbosity,
}

#[derive(Default)]
pub(super) struct Capture {
    state: Mutex<Captured>,
}

impl Capture {
    pub(super) fn with_answers(answers: VecDeque<Expected>) -> Self {
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
    ///
    /// `menu` is the options a menu prompt offers, and `None` for a prompt
    /// that offers none. An answer prepared with the options the menu must
    /// offer fails when the menu offers others, and when the prompt is no
    /// menu: the test was written for a prompt the command does not show.
    fn answer(&self, question: &str, menu: Option<&[String]>) -> Result<String, FrameworkError> {
        let mut captured = self.lock();
        let Some(next) = captured.answers.front() else {
            return Err(FrameworkError::internal(format!(
                "console test: the command asked `{question}`, and the test has no answer \
                 left; add `.expects_question(\"{question}\", ...)`"
            )));
        };
        if next.question != question {
            return Err(FrameworkError::internal(format!(
                "console test: the command asked `{question}`, and the question the test \
                 expects next is `{}`",
                next.question
            )));
        }
        match (&next.options, menu) {
            (Some(_), None) => {
                return Err(FrameworkError::internal(format!(
                    "console test: `{question}` offers no options, and the test expects a \
                     choice; prepare its answer with `.expects_question`"
                )));
            }
            (Some(expected), Some(offered)) if expected.as_slice() != offered => {
                return Err(FrameworkError::internal(format!(
                    "console test: `{question}` offers {}, and the test expects {}",
                    quoted_list(offered),
                    quoted_list(expected)
                )));
            }
            _ => {}
        }
        captured.output.push_str(question);
        captured.output.push('\n');
        Ok(captured
            .answers
            .pop_front()
            .map(|expected| expected.answer)
            .unwrap_or_default())
    }
}

/// `options` as `` `a`, `b` `` for a message.
pub(super) fn quoted_list(options: &[String]) -> String {
    options
        .iter()
        .map(|option| format!("`{option}`"))
        .collect::<Vec<_>>()
        .join(", ")
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

/// How much a command run writes: what `-q` and `-v` to `-vvv` asked for.
///
/// A command writes its routine lines at [`Normal`](Self::Normal) and its
/// detail at a higher level, so the person who runs it chooses how much
/// they read. The order is the order of the variants: `Quiet` is the
/// lowest and `Debug` the highest. Laravel inherits the same five levels
/// from Symfony.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verbosity {
    /// `-q` or `--quiet`: the command writes nothing through
    /// [`line()`], [`error_line`], [`line_at`], [`error_at`], [`error`],
    /// [`warn`], [`info`] or a [`Progress`](super::Progress) bar.
    Quiet,
    /// No flag: the level [`line()`] and [`error_line`] write at.
    #[default]
    Normal,
    /// `-v`: detail an operator reads when something looks wrong.
    Verbose,
    /// `-vv`: more detail than `-v`.
    VeryVerbose,
    /// `-vvv`: everything the command can say.
    Debug,
}

impl Verbosity {
    /// The level of `-v` given `count` times, `-vvv` and more being
    /// [`Debug`](Self::Debug).
    pub(super) fn from_count(count: u8) -> Self {
        match count {
            0 => Self::Normal,
            1 => Self::Verbose,
            2 => Self::VeryVerbose,
            _ => Self::Debug,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Quiet,
            1 => Self::Normal,
            2 => Self::Verbose,
            3 => Self::VeryVerbose,
            _ => Self::Debug,
        }
    }
}

/// The level of a run outside a test. A test keeps its own in its capture,
/// so tests that run at once do not read each other's level.
static PROCESS_VERBOSITY: AtomicU8 = AtomicU8::new(Verbosity::Normal as u8);

/// Record the level the run's flags asked for, for the test that runs it or
/// for the process.
pub(super) fn set_verbosity(level: Verbosity) {
    match capture() {
        Some(capture) => capture.lock().verbosity = level,
        None => PROCESS_VERBOSITY.store(level as u8, Ordering::Relaxed),
    }
}

/// The level of the running command: [`Verbosity::Normal`] unless the
/// command was run with `-q` or `-v` to `-vvv`.
///
/// A command reads it to skip work that only feeds output nobody asked
/// for, such as collecting the detail of every row it skipped.
pub fn verbosity() -> Verbosity {
    match capture() {
        Some(capture) => capture.lock().verbosity,
        None => Verbosity::from_u8(PROCESS_VERBOSITY.load(Ordering::Relaxed)),
    }
}

/// Whether a line asked for at `level` is written in this run. Nothing is
/// written in a quiet run, a line asked for at [`Verbosity::Quiet`]
/// included.
fn writes_at(level: Verbosity) -> bool {
    let current = verbosity();
    current != Verbosity::Quiet && current >= level
}

/// Print one line for the person who ran the command.
///
/// Use this where you would use `println!`. It writes to the standard
/// output when the command runs in the console binary, and a test that
/// runs the command with [`test`](super::testing::test) reads the line
/// from [`ConsoleRun::output`](super::testing::ConsoleRun::output). What
/// `println!` prints, the test cannot see.
///
/// The line is written at [`Verbosity::Normal`], so `-q` silences it.
///
/// ```rust
/// suprnova::console::line(format!("pruned {} rows", 3));
/// ```
pub fn line(text: impl Display) {
    line_at(text, Verbosity::Normal);
}

/// Print one line on the standard error: a warning, or a note that must
/// not end up in output that is piped into another program. A test reads
/// it from [`ConsoleRun::errors`](super::testing::ConsoleRun::errors).
///
/// The line is written at [`Verbosity::Normal`], so `-q` silences it.
///
/// A command that fails returns the error. The console prints the error
/// of a failed command itself, and printing it here as well shows it
/// twice.
pub fn error_line(text: impl Display) {
    error_at(text, Verbosity::Normal);
}

/// Print one line on the standard output when the run asked for at least
/// `level`: [`line()`] for detail that only `-v` and above should show.
///
/// ```rust
/// use suprnova::console::{self, Verbosity};
///
/// console::line_at("checked the archive table", Verbosity::Verbose);
/// ```
pub fn line_at(text: impl Display, level: Verbosity) {
    if writes_at(level) {
        write_output(&format!("{text}\n"));
    }
}

/// Print one line on the standard error when the run asked for at least
/// `level`: [`error_line`] for detail that only `-v` and above should show.
pub fn error_at(text: impl Display, level: Verbosity) {
    if writes_at(level) {
        write_errors(&format!("{text}\n"));
    }
}

/// The marks [`error`], [`warn`] and [`info`] put in front of a line.
#[derive(Clone, Copy)]
enum Mark {
    Error,
    Warn,
    Info,
}

impl Mark {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
        }
    }

    /// White on red, black on yellow and white on blue: the colours of
    /// Laravel's console components.
    fn style(self) -> anstyle::Style {
        let (fg, bg) = match self {
            Self::Error => (anstyle::AnsiColor::White, anstyle::AnsiColor::Red),
            Self::Warn => (anstyle::AnsiColor::Black, anstyle::AnsiColor::Yellow),
            Self::Info => (anstyle::AnsiColor::White, anstyle::AnsiColor::Blue),
        };
        anstyle::Style::new()
            .bold()
            .fg_color(Some(fg.into()))
            .bg_color(Some(bg.into()))
    }

    /// `ERROR` and `WARN` go to the standard error, `INFO` to the output.
    fn goes_to_errors(self) -> bool {
        !matches!(self, Self::Info)
    }
}

/// Whether to style a line on `stream`: only a terminal shows a style,
/// and a `NO_COLOR` that is set asks for none.
fn styled(stream: &impl IsTerminal) -> bool {
    stream.is_terminal() && !no_color_is_set()
}

/// Whether `NO_COLOR` is present in the environment, empty or not. PAR-140
/// reads presence as the request, where <https://no-color.org> ignores an
/// empty value; the framework follows the requirement, so `NO_COLOR=""`
/// turns the styles off.
fn no_color_is_set() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

/// Write `text` behind `mark`, at [`Verbosity::Normal`]. A test reads the
/// plain `ERROR text`; a terminal shows the mark styled.
fn write_marked(mark: Mark, text: impl Display) {
    if !writes_at(Verbosity::Normal) {
        return;
    }
    let plain = format!("{} {text}\n", mark.label());
    if is_captured() {
        if mark.goes_to_errors() {
            write_errors(&plain);
        } else {
            write_output(&plain);
        }
        return;
    }
    let style = mark.style();
    let fancy = format!("{style} {} {style:#} {text}\n", mark.label());
    // As in `write_output`: a failed write has nowhere left to be reported.
    if mark.goes_to_errors() {
        let stream = std::io::stderr();
        if styled(&stream) {
            let _ = anstream::AutoStream::always(stream.lock()).write_all(fancy.as_bytes());
        } else {
            let _ = stream.lock().write_all(plain.as_bytes());
        }
    } else {
        let stream = std::io::stdout();
        if styled(&stream) {
            let _ = anstream::AutoStream::always(stream.lock()).write_all(fancy.as_bytes());
        } else {
            let _ = stream.lock().write_all(plain.as_bytes());
        }
    }
}

/// Print one line marked `ERROR` on the standard error.
///
/// For a problem the command reports and goes on after, such as one row it
/// could not import. A command that fails returns its error instead, and
/// the console prints it.
///
/// The mark is styled when the standard error is a terminal and
/// `NO_COLOR` is not set, and plain text everywhere else: a test reads
/// `ERROR boom` from [`ConsoleRun::errors`](super::testing::ConsoleRun::errors).
pub fn error(text: impl Display) {
    write_marked(Mark::Error, text);
}

/// Print one line marked `WARN` on the standard error, styled as
/// [`error`] styles its mark.
pub fn warn(text: impl Display) {
    write_marked(Mark::Warn, text);
}

/// Print one line marked `INFO` on the standard output, styled as
/// [`error`] styles its mark.
pub fn info(text: impl Display) {
    write_marked(Mark::Info, text);
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
    read_yes_no(question, &answer, default)
}

/// `answer` to the yes-or-no `question`, as [`confirm`] reads it.
pub(super) fn read_yes_no(
    question: &str,
    answer: &str,
    default: bool,
) -> Result<bool, FrameworkError> {
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
    read_answer(question, &format!("{question}{hint} "), None, Echo::Shown)
}

/// Whether the person sees what they type.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Echo {
    Shown,
    /// The terminal does not echo the answer, and the answer is never
    /// written anywhere.
    Hidden,
}

/// The one line that answers `question`.
///
/// In a test, the answer the test prepared, checked against `menu` (see
/// [`Capture::answer`]); the question is a part of the captured output and
/// the answer is not. Outside a test, `prompt` is printed on the standard
/// output and one line is read from the standard input, a terminal or a
/// pipe alike. With [`Echo::Hidden`] and a terminal, the terminal does not
/// echo what is typed.
pub(super) fn read_answer(
    question: &str,
    prompt: &str,
    menu: Option<&[String]>,
    echo: Echo,
) -> Result<String, FrameworkError> {
    if let Some(capture) = capture() {
        return capture.answer(question, menu);
    }

    {
        let mut out = std::io::stdout().lock();
        write!(out, "{prompt}")
            .and_then(|()| out.flush())
            .map_err(|e| {
                FrameworkError::internal(format!("console: cannot print `{question}`: {e}"))
            })?;
    }
    if echo == Echo::Hidden && std::io::stdin().is_terminal() {
        return read_hidden(question);
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

/// One line from the terminal with its echo turned off, through the
/// `console` crate dialoguer re-exports. It turns the echo back on before
/// it returns, and ends the line the person could not see.
fn read_hidden(question: &str) -> Result<String, FrameworkError> {
    use dialoguer::console::Term;
    let term = if std::io::stderr().is_terminal() {
        Term::stderr()
    } else if std::io::stdout().is_terminal() {
        Term::stdout()
    } else {
        // `read_secure_line` needs a terminal to write to, and reading the
        // answer with the echo on would show it.
        return Err(FrameworkError::bad_request(format!(
            "`{question}` was not answered: a hidden answer needs a terminal to read it from, \
             or the answer piped in"
        )));
    };
    term.read_secure_line().map_err(|e| {
        FrameworkError::internal(format!(
            "console: cannot read the answer to `{question}`: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collected<F: Future>(answers: &[(&str, &str)], future: F) -> (F::Output, Captured) {
        let capture = Arc::new(Capture::with_answers(
            answers
                .iter()
                .map(|(q, a)| Expected {
                    question: (*q).to_owned(),
                    answer: (*a).to_owned(),
                    options: None,
                })
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

    // The environment is process-wide; only this test writes NO_COLOR.
    static NO_COLOR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn no_color_present_but_empty_still_turns_the_styles_off() {
        let _guard = NO_COLOR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::env::var_os("NO_COLOR");

        // SAFETY: NO_COLOR_LOCK serializes every access to NO_COLOR in this binary.
        unsafe { std::env::set_var("NO_COLOR", "") };
        let present_but_empty = no_color_is_set();
        // SAFETY: as above.
        unsafe { std::env::remove_var("NO_COLOR") };
        let unset = no_color_is_set();
        // SAFETY: as above.
        unsafe {
            match before {
                Some(value) => std::env::set_var("NO_COLOR", value),
                None => std::env::remove_var("NO_COLOR"),
            }
        }

        assert!(
            present_but_empty,
            "PAR-140: NO_COLOR present but empty must turn the styles off"
        );
        assert!(!unset, "PAR-140: NO_COLOR unset must leave the styles on");
    }
}
