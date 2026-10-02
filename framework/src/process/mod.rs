//! Running other programs: the `Process` facade.
//!
//! [`Process::command`] runs a program with its arguments, each passed to
//! it as it is, through no shell. [`Process::shell`] runs a command line
//! through the system shell, the way Laravel runs a string command, for
//! pipes, redirects and globs. The two are kept apart so a value from
//! outside never reaches a shell unless the code asks for one.
//!
//! [`PendingProcess::run`] waits for the program and returns a
//! [`ProcessResult`]: the exit code and the captured output. A nonzero exit
//! is a result that reports failure, not an error; [`ProcessResult::throw`]
//! turns it into one. [`PendingProcess::start`] returns an
//! [`InvokedProcess`] to watch, signal, stop or wait on.
//!
//! # Cleanup
//!
//! On Unix each process gets a process group of its own, and every kill
//! reaches the whole group, so the processes a command started die with
//! it: on a timeout, an idle timeout, a `stop`, and when a started process
//! or the future of `run` is dropped before it completes. A process that
//! exits normally is waited on until its output closes, as Laravel waits,
//! and is not killed. Elsewhere only the program itself is killed.
//!
//! # Pools and pipes
//!
//! [`Process::pool`] runs processes side by side, at most
//! [`Pool::concurrency`] at once when that is set. [`Process::pipe`] runs
//! them in order, each with the previous one's output as its input.
//!
//! # Fakes
//!
//! With the `testing` feature, [`Process::fake`] stops every process from
//! running while its guard lives and records each one for the assertions
//! on [`ProcessFake`]. The fake is process-global, like `Storage::fake`,
//! and the guard serializes the tests that take one.

#[cfg(any(test, feature = "testing"))]
mod fake;
mod invoked;
mod pool;

#[cfg(any(test, feature = "testing"))]
pub use fake::{
    FakeDescription, FakeHandler, FakeResult, FakeSequence, ProcessFake, RecordedProcess,
};
pub use invoked::InvokedProcess;
pub use pool::{InvokedPool, Pipe, Pool, PoolResults};

use crate::error::FrameworkError;
use std::path::PathBuf;
use std::time::Duration;

/// The timeout a process gets unless it sets another: 60 seconds, as in
/// Laravel.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// The facade. Every method builds something to run; nothing runs until
/// `run` or `start` is called on it.
pub struct Process;

impl Process {
    /// A program and its arguments: `args[0]` is the program, found on
    /// `PATH`, and the rest reach it as they are, through no shell.
    ///
    /// ```rust,no_run
    /// # async fn ex() -> Result<(), suprnova::ProcessError> {
    /// use suprnova::Process;
    ///
    /// let status = Process::command(["git", "status", "--short"])
    ///     .path("/srv/app")
    ///     .run()
    ///     .await?;
    /// println!("{}", status.output());
    /// # Ok(()) }
    /// ```
    pub fn command<I, S>(args: I) -> PendingProcess
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        PendingProcess::new(Command::Args(args.into_iter().map(Into::into).collect()))
    }

    /// A command line run through the system shell, `sh -c` (`cmd /C` on
    /// Windows), the way Laravel runs a string command. Pipes, `&&`,
    /// redirects, `$VAR` and globs work. Never build the line from input
    /// you do not control: the shell runs whatever it holds. Use
    /// [`Process::command`] for that.
    pub fn shell(line: impl Into<String>) -> PendingProcess {
        PendingProcess::new(Command::Shell(line.into()))
    }

    /// Whether this process has a terminal on standard input and standard
    /// output, so [`PendingProcess::tty`] can hand it on.
    pub fn supports_tty() -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    /// A pool of processes run side by side. See [`Pool`].
    pub fn pool() -> Pool {
        Pool::new()
    }

    /// A pipe of processes run in order, each fed the previous one's
    /// output. See [`Pipe`].
    pub fn pipe() -> Pipe {
        Pipe::new()
    }

    /// Stop every process from running while the returned guard lives, and
    /// record each one. See [`ProcessFake`].
    #[cfg(any(test, feature = "testing"))]
    pub fn fake() -> ProcessFake {
        fake::install()
    }

    /// A faked result with this standard output, exit code 0 unless set.
    #[cfg(any(test, feature = "testing"))]
    pub fn result(output: impl Into<String>) -> FakeResult {
        FakeResult::new(output)
    }

    /// A faked process described line by line, which can run for a number
    /// of `running` checks. See [`FakeDescription`].
    #[cfg(any(test, feature = "testing"))]
    pub fn describe() -> FakeDescription {
        FakeDescription::default()
    }

    /// Faked results answered in turn, one a run. See [`FakeSequence`].
    #[cfg(any(test, feature = "testing"))]
    pub fn sequence<I, H>(results: I) -> FakeSequence
    where
        I: IntoIterator<Item = H>,
        H: Into<FakeHandler>,
    {
        FakeSequence::new(results)
    }
}

/// Which stream a chunk of output came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputKind {
    /// Standard output.
    Out,
    /// Standard error.
    Err,
}

/// A signal for [`InvokedProcess::signal`] and [`InvokedPool::signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    /// `SIGTERM`: asks the process to end.
    Term,
    /// `SIGKILL`: ends it.
    Kill,
    /// `SIGINT`: an interrupt, as Ctrl-C sends.
    Int,
    /// `SIGHUP`.
    Hup,
    /// `SIGQUIT`.
    Quit,
    /// `SIGUSR1`.
    Usr1,
    /// `SIGUSR2`.
    Usr2,
}

/// What runs.
#[derive(Debug, Clone)]
pub(crate) enum Command {
    /// A program and its arguments.
    Args(Vec<String>),
    /// A line for the shell.
    Shell(String),
}

impl Command {
    /// The command line: the arguments joined by spaces, or the shell line
    /// as given. Fakes match it and results name it.
    pub(crate) fn line(&self) -> String {
        match self {
            Command::Args(args) => args.join(" "),
            Command::Shell(line) => line.clone(),
        }
    }

    /// The program that is started.
    pub(crate) fn program(&self) -> String {
        match self {
            Command::Args(args) => args.first().cloned().unwrap_or_default(),
            Command::Shell(_) => shell_program().to_owned(),
        }
    }

    pub(crate) fn to_tokio(&self) -> tokio::process::Command {
        match self {
            Command::Args(args) => {
                let mut command =
                    tokio::process::Command::new(args.first().map_or("", String::as_str));
                command.args(args.iter().skip(1));
                command
            }
            Command::Shell(line) => {
                let mut command = tokio::process::Command::new(shell_program());
                command.arg(shell_flag()).arg(line);
                command
            }
        }
    }
}

#[cfg(windows)]
fn shell_program() -> &'static str {
    "cmd"
}

#[cfg(not(windows))]
fn shell_program() -> &'static str {
    "sh"
}

#[cfg(windows)]
fn shell_flag() -> &'static str {
    "/C"
}

#[cfg(not(windows))]
fn shell_flag() -> &'static str {
    "-c"
}

/// A process to run, with its settings. Every setter takes and returns the
/// builder, so a process reads as one chain.
#[derive(Debug, Clone)]
pub struct PendingProcess {
    pub(crate) command: Command,
    pub(crate) path: Option<PathBuf>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) input: Option<Vec<u8>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) idle_timeout: Option<Duration>,
    pub(crate) quietly: bool,
    pub(crate) tty: bool,
}

impl PendingProcess {
    fn new(command: Command) -> Self {
        Self {
            command,
            path: None,
            env: Vec::new(),
            input: None,
            timeout: Some(DEFAULT_TIMEOUT),
            idle_timeout: None,
            quietly: false,
            tty: false,
        }
    }

    /// Run in this working directory.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Add a variable to the environment the process inherits.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Write these bytes to the process's standard input, then close it.
    /// Without input, standard input is empty.
    pub fn input(mut self, input: impl Into<Vec<u8>>) -> Self {
        self.input = Some(input.into());
        self
    }

    /// Kill the process, with every process it started, when it runs
    /// longer than this. The default is [`DEFAULT_TIMEOUT`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Kill the process, with every process it started, when it writes no
    /// output for this long.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    /// Run with no timeout.
    pub fn forever(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Keep the output out of the result and call no output callback.
    pub fn quietly(mut self) -> Self {
        self.quietly = true;
        self
    }

    /// Hand the process this terminal's standard input and output, for a
    /// program that talks to the user. Nothing is captured, and the process
    /// stays in the terminal's process group, so a kill reaches only the
    /// program itself.
    pub fn tty(mut self) -> Self {
        self.tty = true;
        self
    }

    /// The command line: the arguments joined by spaces, or the shell line
    /// as given.
    pub fn command_line(&self) -> String {
        self.command.line()
    }

    /// Run the process and wait for it.
    ///
    /// # Errors
    ///
    /// [`ProcessError::NotStarted`] when the program cannot be started,
    /// [`ProcessError::TimedOut`] and [`ProcessError::IdleTimedOut`] when it
    /// is killed for running too long or going quiet, and, under a fake,
    /// [`ProcessError::Stray`] and [`ProcessError::FakeExhausted`]. A
    /// nonzero exit is not an error.
    pub async fn run(self) -> Result<ProcessResult, ProcessError> {
        self.start()?.wait().await
    }

    /// [`run`](Self::run), with `output` called with each chunk of standard
    /// output and standard error as it arrives.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run).
    pub async fn run_with<F>(self, output: F) -> Result<ProcessResult, ProcessError>
    where
        F: FnMut(OutputKind, &str) + Send + 'static,
    {
        self.start_with(output)?.wait().await
    }

    /// Start the process and return it running. Call inside a Tokio
    /// runtime: the output is read by tasks of its own.
    ///
    /// # Errors
    ///
    /// [`ProcessError::NotStarted`] when the program cannot be started, and
    /// under a fake [`ProcessError::Stray`] and
    /// [`ProcessError::FakeExhausted`].
    pub fn start(self) -> Result<InvokedProcess, ProcessError> {
        InvokedProcess::start(self, None)
    }

    /// [`start`](Self::start), with `output` called with each chunk of
    /// output as it arrives.
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start).
    pub fn start_with<F>(self, output: F) -> Result<InvokedProcess, ProcessError>
    where
        F: FnMut(OutputKind, &str) + Send + 'static,
    {
        InvokedProcess::start(self, Some(Box::new(output)))
    }
}

/// What a process did: its exit code and the output it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResult {
    pub(crate) command: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) output: String,
    pub(crate) error_output: String,
}

impl ProcessResult {
    /// The command line that ran.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Whether the process exited with code 0.
    pub fn successful(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Whether it did not: a nonzero exit, or a kill by a signal.
    pub fn failed(&self) -> bool {
        !self.successful()
    }

    /// The exit code; `None` when a signal ended the process.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Everything the process wrote to standard output. Bytes that are not
    /// UTF-8 are replaced.
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Everything the process wrote to standard error.
    pub fn error_output(&self) -> &str {
        &self.error_output
    }

    /// Whether standard output contains `text`.
    pub fn see_in_output(&self, text: &str) -> bool {
        self.output.contains(text)
    }

    /// Whether standard error contains `text`.
    pub fn see_in_error_output(&self, text: &str) -> bool {
        self.error_output.contains(text)
    }

    /// The result when it succeeded.
    ///
    /// # Errors
    ///
    /// [`ProcessError::Failed`], carrying the result, when it failed.
    pub fn throw(self) -> Result<Self, ProcessError> {
        if self.successful() {
            Ok(self)
        } else {
            Err(ProcessError::Failed {
                result: Box::new(self),
            })
        }
    }

    /// [`throw`](Self::throw) when `condition` holds, the result otherwise.
    ///
    /// # Errors
    ///
    /// As [`throw`](Self::throw).
    pub fn throw_if(self, condition: bool) -> Result<Self, ProcessError> {
        if condition { self.throw() } else { Ok(self) }
    }
}

/// Why a process did not give a result.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The program could not be started: it is not on `PATH`, is not
    /// executable, or the working directory does not exist.
    #[error("the program '{program}' could not be started: {source}")]
    NotStarted {
        /// The program.
        program: String,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// The process ran past its timeout and was killed, with every process
    /// it started.
    #[error("the process \"{command}\" exceeded the timeout of {}", describe_duration(*timeout))]
    TimedOut {
        /// The command line.
        command: String,
        /// The timeout.
        timeout: Duration,
        /// What the process wrote before the kill.
        result: Box<ProcessResult>,
    },
    /// The process wrote nothing for its idle timeout and was killed, with
    /// every process it started.
    #[error("the process \"{command}\" wrote no output for {}", describe_duration(*timeout))]
    IdleTimedOut {
        /// The command line.
        command: String,
        /// The idle timeout.
        timeout: Duration,
        /// What the process wrote before the kill.
        result: Box<ProcessResult>,
    },
    /// [`ProcessResult::throw`] on a failed result.
    #[error(
        "the process \"{}\" failed with {}\n\nOutput:\n{}\n\nError output:\n{}",
        result.command,
        describe_exit(result.exit_code),
        result.output,
        result.error_output
    )]
    Failed {
        /// The failed result.
        result: Box<ProcessResult>,
    },
    /// Under a fake that prevents stray processes, no pattern matched.
    #[error("attempted to run the process \"{command}\" without a matching fake")]
    Stray {
        /// The command line.
        command: String,
    },
    /// The faked sequence for the command had no result left.
    #[error("the faked sequence for \"{command}\" has no result left")]
    FakeExhausted {
        /// The command line.
        command: String,
    },
    /// A signal could not be sent.
    #[error("could not signal the process \"{command}\": {message}")]
    Signal {
        /// The command line.
        command: String,
        /// Why.
        message: String,
    },
    /// Reading from or waiting on the process failed.
    #[error("the process \"{command}\" failed: {source}")]
    Io {
        /// The command line.
        command: String,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

impl From<ProcessError> for FrameworkError {
    fn from(error: ProcessError) -> Self {
        FrameworkError::internal(error.to_string())
    }
}

fn describe_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        match duration.as_secs() {
            1 => "1 second".to_owned(),
            seconds => format!("{seconds} seconds"),
        }
    } else {
        format!("{} milliseconds", duration.as_millis())
    }
}

fn describe_exit(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exit code {code}"),
        None => "no exit code: a signal ended it".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_sync<T: Send + Sync>() {}

    #[test]
    fn the_public_types_cross_threads() {
        send_sync::<PendingProcess>();
        send_sync::<InvokedProcess>();
        send_sync::<ProcessResult>();
        send_sync::<ProcessError>();
        send_sync::<Pool>();
        send_sync::<InvokedPool>();
        send_sync::<PoolResults>();
        send_sync::<Pipe>();
    }

    #[test]
    fn durations_read_as_people_write_them() {
        assert_eq!(describe_duration(Duration::from_secs(1)), "1 second");
        assert_eq!(describe_duration(Duration::from_secs(60)), "60 seconds");
        assert_eq!(
            describe_duration(Duration::from_millis(1500)),
            "1500 milliseconds"
        );
    }

    #[test]
    fn the_command_line_joins_arguments_and_keeps_a_shell_line() {
        assert_eq!(
            Process::command(["git", "status"]).command_line(),
            "git status"
        );
        assert_eq!(Process::shell("a | b").command_line(), "a | b");
    }

    #[test]
    fn a_failed_result_throws_with_its_outputs() {
        let result = ProcessResult {
            command: "x".into(),
            exit_code: Some(2),
            output: "o".into(),
            error_output: "e".into(),
        };
        let text = result.throw().unwrap_err().to_string();
        assert!(text.contains("exit code 2") && text.contains("o") && text.contains("e"));
    }
}
