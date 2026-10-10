//! The tasks the tests run, registered in this binary by
//! `#[concurrent_task]`, so the process driver's console process (this
//! binary again) finds them by name.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
