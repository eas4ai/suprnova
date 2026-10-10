//! The `process` driver: each task in a console process of its own.

use super::driver::{ConcurrencyDriver, timed_out};
use super::task::{Task, TaskResult};
use crate::error::FrameworkError;
use crate::process::{Process, ProcessError, ProcessResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Duration;

/// The hidden console command that runs one task in a console process.
pub(crate) const RUN_COMMAND: &str = "concurrency:run";

/// The variable that carries a run's token to the console process. The
/// driver sets it for every console process it starts; [`RESULT_MARKER`]
/// says why.
pub(crate) const TOKEN_ENV: &str = "SUPRNOVA_CONCURRENCY_TOKEN";

/// The start of the line that carries a task's outcome on the console
/// process's standard output.
///
/// The line is `{RESULT_MARKER}{token}:{json}`, where the token is the one
/// the driver generated for this run and passed in [`TOKEN_ENV`]. The
/// application's own logging shares standard output with the outcome: a
/// stdout log channel, or a queued listener still running while the
/// console shuts down, can write text that starts with this marker, before
/// the outcome line or after it. The driver takes only the line that
/// carries its run's token, which no logged text knows, so a logged line
/// that reads as an outcome is never taken. Without the variable, as when
/// someone runs `concurrency:run` by hand, the line is
/// `{RESULT_MARKER}{json}`.
pub(crate) const RESULT_MARKER: &str = "suprnova-concurrency-result:";

/// A task's outcome as the console process writes it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Envelope {
    /// The task's output.
    Ok(Value),
    /// Why the task failed, as its error displays.
    Error(String),
}

/// The program and leading arguments set with
/// [`Concurrency::console_command`](super::Concurrency::console_command).
static CONSOLE_COMMAND: RwLock<Option<Vec<String>>> = RwLock::new(None);

/// Runs each task in a console process of its own, through the
/// [`Process`] facade, as Laravel's `process` driver runs each closure in
/// an `artisan` process.
///
/// The console binary is started with `concurrency:run -- <task>`, the
/// task's input as JSON on its standard input and a random token for the
/// run in `SUPRNOVA_CONCURRENCY_TOKEN`. The hidden command finds the task
/// in that binary's registry, runs it, and writes the outcome, with the
/// token, on its standard output, which this driver reads back. The tasks
/// run side by side, each with the isolation of a process: its own memory,
/// its own runtime, and a timeout that kills it and every process it
/// started.
///
/// The binary is the one
/// [`Concurrency::console_command`](super::Concurrency::console_command)
/// or [`Concurrency::console_binary`](super::Concurrency::console_binary)
/// set; otherwise the `console` binary beside the running executable, as
/// a Suprnova project builds them side by side; otherwise the running
/// executable itself, which is right when the console binary is the one
/// that runs the tasks. Starting a process costs far more than a Tokio
/// task, so this driver pays off for heavy or untrusted work.
///
/// The outcome shares standard output with the application's own logging,
/// which can write a line that reads as an outcome, before the outcome line
/// or after it. The driver therefore takes only the outcome line that
/// carries the token it gave the process, takes a success only from a
/// console process that ended with exit code 0, and refuses a task name
/// with a control character, so a name cannot start a line of its own
/// there.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessConcurrencyDriver;

#[async_trait]
impl ConcurrencyDriver for ProcessConcurrencyDriver {
    async fn run(
        &self,
        tasks: Vec<Task>,
        timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        let command = match console_command() {
            Ok(command) => command,
            Err(error) => return tasks.iter().map(|_| Err(error.clone())).collect(),
        };
        futures::future::join_all(
            tasks
                .into_iter()
                .map(|task| run_one(&command, task, timeout)),
        )
        .await
    }
}

/// Run one task in a console process and read its outcome.
async fn run_one(
    command: &[String],
    task: Task,
    timeout: Option<Duration>,
) -> Result<TaskResult, FrameworkError> {
    let name = task.name().to_owned();
    let input = serde_json::to_vec(task.input()?).map_err(|error| {
        FrameworkError::internal(format!(
            "concurrent task `{name}`: its input could not be serialized: {error}"
        ))
    })?;
    // `--` keeps a name that starts with a dash from reading as an option.
    let arguments =
        command
            .iter()
            .cloned()
            .chain([RUN_COMMAND.to_owned(), "--".to_owned(), name.clone()]);
    let token = new_token();
    let pending = Process::command(arguments)
        .env(TOKEN_ENV, token.as_str())
        .input(input);
    // The facade's "no timeout" is no timeout here too, not the Process
    // facade's default of 60 seconds.
    let pending = match timeout {
        Some(timeout) => pending.timeout(timeout),
        None => pending.forever(),
    };
    match pending.run().await {
        Ok(result) => read_outcome(&name, &token, &result),
        Err(ProcessError::TimedOut { timeout, .. }) => Err(timed_out(&name, timeout)),
        Err(error) => Err(FrameworkError::internal(format!(
            "concurrent task `{name}` could not run in a console process: {error}"
        ))),
    }
}

/// A fresh token for one console process: 128 bits from the thread's
/// cryptographically secure generator, as 32 hex digits. It is random so
/// that no text the application logs can carry it, by chance or by craft.
fn new_token() -> String {
    hex::encode(rand::random::<[u8; 16]>())
}

/// The outcome the console process wrote, or an error naming the task
/// when it wrote none or did not succeed.
///
/// The outcome is the last line that starts with [`RESULT_MARKER`],
/// `token` and a colon; every other marked line is ignored. The
/// application's own logging shares standard output with the outcome, and
/// a logged text that reads as an outcome, before the outcome line or
/// after it, must not be taken: it cannot carry the token, which the driver
/// generated for this run alone. The exit status is checked too:
/// `concurrency:run` ends with exit code 1 whenever the task fails, so a
/// success counts only from a process that ended with exit code 0.
fn read_outcome(
    name: &str,
    token: &str,
    result: &ProcessResult,
) -> Result<TaskResult, FrameworkError> {
    let envelope = result.output().lines().rev().find_map(|line| {
        line.strip_prefix(RESULT_MARKER)?
            .strip_prefix(token)?
            .strip_prefix(':')
    });
    match envelope.map(serde_json::from_str::<Envelope>) {
        Some(Ok(Envelope::Ok(value))) if result.successful() => Ok(TaskResult::new(name, value)),
        Some(Ok(Envelope::Ok(_))) => Err(FrameworkError::internal(format!(
            "concurrent task `{name}` did not succeed: `{}` ended with {} after a line that \
             reads as a success outcome, and only a console process that ends with exit \
             code 0 answers a success{}",
            result.command(),
            ended(result),
            error_output(result)
        ))),
        Some(Ok(Envelope::Error(message))) => Err(FrameworkError::internal(format!(
            "concurrent task `{name}` failed: {message}"
        ))),
        Some(Err(error)) => Err(FrameworkError::internal(format!(
            "concurrent task `{name}` wrote an outcome that does not decode: {error}"
        ))),
        None => Err(FrameworkError::internal(format!(
            "concurrent task `{name}` wrote no outcome: `{}` ended with {}; the console \
             binary must be the application's, which registers `{RUN_COMMAND}`{}",
            result.command(),
            ended(result),
            error_output(result)
        ))),
    }
}

/// How the console process ended, for an error: its exit code, or a signal.
fn ended(result: &ProcessResult) -> String {
    match result.exit_code() {
        Some(code) => format!("exit code {code}"),
        None => "a signal".to_owned(),
    }
}

/// The console process's error output as the end of an error, or nothing
/// when it wrote none.
fn error_output(result: &ProcessResult) -> String {
    let errors = result.error_output().trim();
    if errors.is_empty() {
        String::new()
    } else {
        format!("; its error output: {errors}")
    }
}

/// Set the program and leading arguments the driver starts.
pub(super) fn set_console_command(command: Vec<String>) -> Result<(), FrameworkError> {
    *crate::lock::write(&CONSOLE_COMMAND, "concurrency console command")? = Some(command);
    Ok(())
}

/// The program and leading arguments the driver starts, before
/// `concurrency:run <task>`.
pub(super) fn console_command() -> Result<Vec<String>, FrameworkError> {
    if let Some(command) = crate::lock::read(&CONSOLE_COMMAND, "concurrency console command")?
        .as_ref()
        .cloned()
    {
        return Ok(command);
    }
    let current = std::env::current_exe().map_err(|error| {
        FrameworkError::internal(format!(
            "the process concurrency driver could not find the running executable: {error}; \
             set the console binary with Concurrency::console_binary"
        ))
    })?;
    let sibling = current.with_file_name(format!("console{}", std::env::consts::EXE_SUFFIX));
    let program = if sibling != current && sibling.is_file() {
        sibling
    } else {
        current
    };
    Ok(vec![path_text(program)?])
}

/// The path as text, which the Process facade takes.
pub(super) fn path_text(path: PathBuf) -> Result<String, FrameworkError> {
    path.into_os_string().into_string().map_err(|path| {
        FrameworkError::internal(format!(
            "the console binary path {path:?} is not UTF-8; the process concurrency driver \
             cannot start it"
        ))
    })
}
