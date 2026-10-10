//! `concurrency:run` - the hidden console command the process driver
//! starts, which runs one registered task in this process.
//!
//! Laravel's process driver starts `artisan invoke-serialized-closure`
//! with the closure serialized in the environment. A Rust closure cannot be
//! serialized, so this command takes the registered name of a task as its
//! argument and the task's input as JSON on its standard input. It writes
//! the outcome as one marked line on its standard output, which the
//! driver reads back, and ends with exit code 1 when the task fails.
//!
//! The command is hidden from the console's help, as Laravel hides its
//! command: it is a protocol between two processes, not a command for
//! people. It is registered the way the other built-in hidden commands
//! are, with an `inventory::submit!` of a [`CommandEntry`], so every
//! console binary that links the framework has it.

use super::process::{Envelope, RESULT_MARKER, RUN_COMMAND};
use super::task;
use crate::console::CommandEntry;
use crate::error::FrameworkError;
use futures::FutureExt;
use serde_json::Value;
use std::future::Future;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use tokio::io::AsyncReadExt;

/// The id of the command's one argument.
const TASK_ARGUMENT: &str = "task";

fn build_command() -> clap::Command {
    clap::Command::new(RUN_COMMAND)
        .hide(true)
        .about(
            "Run one concurrent task: its JSON input on standard input, its outcome on \
             standard output",
        )
        .arg(
            clap::Arg::new(TASK_ARGUMENT)
                .required(true)
                .help("The registered name of the task"),
        )
}

fn run_command(
    matches: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    // Required by the clap command, so clap has refused argv without it.
    let name = matches
        .get_one::<String>(TASK_ARGUMENT)
        .cloned()
        .unwrap_or_default();
    Box::pin(run(name))
}

inventory::submit! {
    CommandEntry {
        name: RUN_COMMAND,
        description: "Run one concurrent task for the process driver",
        clap_builder: build_command,
        handler: run_command,
    }
}

/// Run the task and write its outcome. A failed task fails the command
/// too, so the console reports the error and ends with exit code 1, as for
/// any failed command.
async fn run(name: String) -> Result<(), FrameworkError> {
    let (envelope, result) = match execute(&name).await {
        Ok(value) => (Envelope::Ok(value), Ok(())),
        Err(error) => (Envelope::Error(error.to_string()), Err(error)),
    };
    write_outcome(&envelope)?;
    result
}

/// Read the input, find the task and run it.
async fn execute(name: &str) -> Result<Value, FrameworkError> {
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| {
            FrameworkError::internal(format!(
                "concurrent task `{name}`: its input could not be read from standard input: {error}"
            ))
        })?;
    let input = if bytes.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|error| {
            FrameworkError::internal(format!(
                "concurrent task `{name}`: its input on standard input is not JSON: {error}"
            ))
        })?
    };
    let entry = task::find(name)?;
    match AssertUnwindSafe(entry.run(input)).catch_unwind().await {
        Ok(outcome) => outcome,
        Err(_) => Err(task::panicked(name)),
    }
}

/// Write the outcome as one marked line. The line starts on a line of its
/// own, whatever the task wrote before it.
fn write_outcome(envelope: &Envelope) -> Result<(), FrameworkError> {
    let json = serde_json::to_string(envelope).map_err(|error| {
        FrameworkError::internal(format!(
            "the outcome of a concurrent task could not be serialized: {error}"
        ))
    })?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(format!("\n{RESULT_MARKER}{json}\n").as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| {
            FrameworkError::internal(format!(
                "the outcome of a concurrent task could not be written to standard output: {error}"
            ))
        })
}
