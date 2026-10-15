//! The tasks the tests run, registered in this binary by
//! `#[concurrent_task]`, so the process driver's console process (this
//! binary again) finds them by name.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use suprnova::concurrency::Task;
use suprnova::{Concurrency, FrameworkError, concurrent_task};

/// The input of [`nap_then_echo`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nap {
    /// How long to sleep.
    pub millis: u64,
    /// What to answer afterwards.
    pub value: String,
}

impl Nap {
    /// A nap of `millis` answering `value`.
    pub fn new(millis: u64, value: &str) -> Self {
        Self {
            millis,
            value: value.to_owned(),
        }
    }
}

/// Sleeps, then answers its value. Two side by side show whether a driver
/// runs tasks at once, and their values show the order of the results.
#[concurrent_task]
pub async fn nap_then_echo(input: Nap) -> Result<String, FrameworkError> {
    tokio::time::sleep(Duration::from_millis(input.millis)).await;
    Ok(input.value)
}

/// A task with no parameter.
#[concurrent_task]
pub async fn answer() -> Result<u64, FrameworkError> {
    Ok(42)
}

/// A task registered under a name of its own.
#[concurrent_task(name = "concurrency-tests:greet", type_name = "Greeting")]
pub async fn greet(name: String) -> Result<String, FrameworkError> {
    Ok(format!("hello {name}"))
}

/// Fails with `message`.
#[concurrent_task]
pub async fn fail_with(message: String) -> Result<(), FrameworkError> {
    Err(FrameworkError::internal(message))
}

/// Panics.
#[concurrent_task]
pub async fn panic_on_purpose() -> Result<(), FrameworkError> {
    panic!("the task panicked on purpose")
}

/// The input of [`write_pid_then_nap`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PidThenNap {
    /// Where to write the process id.
    pub pid_file: PathBuf,
    /// How long to sleep afterwards.
    pub millis: u64,
}

/// Writes the id of the process it runs in, then sleeps, so a test can
/// watch whether a timeout ended that process.
#[concurrent_task]
pub async fn write_pid_then_nap(input: PidThenNap) -> Result<(), FrameworkError> {
    std::fs::write(&input.pid_file, std::process::id().to_string())
        .map_err(|error| FrameworkError::internal(error.to_string()))?;
    tokio::time::sleep(Duration::from_millis(input.millis)).await;
    Ok(())
}

/// The input of [`touch_after`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Touch {
    /// The file to write.
    pub path: PathBuf,
    /// How long to sleep before writing it.
    pub after_millis: u64,
}

/// Sleeps, then writes a file, so a test sees when a deferred task ran,
/// in this process or another.
#[concurrent_task]
pub async fn touch_after(input: Touch) -> Result<(), FrameworkError> {
    tokio::time::sleep(Duration::from_millis(input.after_millis)).await;
    std::fs::write(&input.path, "ran").map_err(|error| FrameworkError::internal(error.to_string()))
}

/// What [`report_default_driver`] answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverReport {
    /// The default driver's name, as the process sees it.
    pub driver: String,
    /// How long two one-second naps took on the default driver.
    pub elapsed_millis: u64,
}

/// Runs two one-second naps on the default driver of the process it runs
/// in and reports which driver that is and how long they took. Run in a
/// console process with `CONCURRENCY_DRIVER` set, it shows what the
/// variable selects.
#[concurrent_task]
pub async fn report_default_driver() -> Result<DriverReport, FrameworkError> {
    let started = Instant::now();
    Concurrency::run([
        Task::new::<NapThenEcho>(Nap::new(1000, "a")),
        Task::new::<NapThenEcho>(Nap::new(1000, "b")),
    ])
    .await?;
    Ok(DriverReport {
        driver: Concurrency::default_driver(),
        elapsed_millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// A task whose registered name starts with dashes, which the console
/// process must read as the name and not as an option.
#[concurrent_task(name = "--dashed-task")]
pub async fn dashed() -> Result<String, FrameworkError> {
    Ok("dashed".to_owned())
}

/// The input of [`stamp_then_nap`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StampNap {
    /// The label the task's stamps are kept under, one per test.
    pub label: String,
    /// How long to sleep between the start and the finish.
    pub millis: u64,
}

/// When a [`stamp_then_nap`] task started and finished.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stamps {
    /// When the task started.
    pub started: Option<Instant>,
    /// When the task finished, after its nap.
    pub finished: Option<Instant>,
}

/// Every [`stamp_then_nap`] task's stamps in this process, by label.
static STAMPS: Mutex<Vec<(String, Stamps)>> = Mutex::new(Vec::new());

/// The stamps of the task labelled `label`; both are `None` until it starts.
pub fn stamps(label: &str) -> Stamps {
    STAMPS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(held, _)| held == label)
        .map(|(_, stamps)| *stamps)
        .unwrap_or_default()
}

/// Change the stamps of the task labelled `label`.
fn stamp(label: &str, change: impl FnOnce(&mut Stamps)) {
    let mut all = STAMPS.lock().unwrap_or_else(PoisonError::into_inner);
    match all.iter_mut().find(|(held, _)| held == label) {
        Some((_, stamps)) => change(stamps),
        None => {
            let mut stamps = Stamps::default();
            change(&mut stamps);
            all.push((label.to_owned(), stamps));
        }
    }
}

/// Records when it starts, sleeps, then records when it finishes, so a
/// test can order a deferred task against the handler that deferred it
/// and the response the client received. It runs in this process, on the
/// tokio or sync driver.
#[concurrent_task]
pub async fn stamp_then_nap(input: StampNap) -> Result<(), FrameworkError> {
    stamp(&input.label, |stamps| stamps.started = Some(Instant::now()));
    tokio::time::sleep(Duration::from_millis(input.millis)).await;
    stamp(&input.label, |stamps| {
        stamps.finished = Some(Instant::now())
    });
    Ok(())
}

/// The line [`forge_success_then_exit`] writes: it reads as a success
/// outcome whose value is `forged-success`.
pub const FORGED_OUTCOME: &str = "suprnova-concurrency-result:{\"ok\":\"forged-success\"}";

/// Writes a line that reads as a success outcome, then ends its process
/// with exit code 3, as a console process does when its own logging repeats
/// a crafted line after the task failed. Run it on the process driver only:
/// in-process it ends the process it runs in.
#[concurrent_task]
pub async fn forge_success_then_exit() -> Result<(), FrameworkError> {
    {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(format!("\n{FORGED_OUTCOME}\n").as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| FrameworkError::internal(error.to_string()))?;
    }
    std::process::exit(3)
}

/// A line that reads as a success outcome with no token, as text an
/// application logs to standard output could.
pub const FORGED_UNTOKENED: &str = "suprnova-concurrency-result:{\"ok\":\"forged\"}";

/// A line that reads as a success outcome carrying a token of another run.
pub const FORGED_OTHER_TOKEN: &str = "suprnova-concurrency-result:00000000000000000000000000000000:{\"ok\":\"forged-with-another-token\"}";

/// Lines a task left for the console to write once its command has
/// returned, after the outcome line. An application's console does that
/// when its logging writes to standard output during the shutdown drain,
/// as a queued listener still running at the end of a command can.
static AFTER_THE_COMMAND: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Write the lines tasks left for after the command on standard output,
/// in the order they were left. `child::entry` calls it once the command
/// has returned, so the lines always come after the outcome line.
pub fn write_lines_left_for_after_the_command() {
    let lines = std::mem::take(
        &mut *AFTER_THE_COMMAND
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let mut stdout = std::io::stdout().lock();
    for line in lines {
        stdout
            .write_all(format!("{line}\n").as_bytes())
            .expect("the line is written");
    }
    stdout.flush().expect("standard output is flushed");
}

/// Answers `genuine`, and leaves two lines that read as success outcomes
/// for the console to write after its own outcome line: one with the token
/// of another run, then one with no token at all.
#[concurrent_task]
pub async fn answer_then_log_forged_outcomes() -> Result<String, FrameworkError> {
    AFTER_THE_COMMAND
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend([FORGED_OTHER_TOKEN.to_owned(), FORGED_UNTOKENED.to_owned()]);
    Ok("genuine".to_owned())
}
