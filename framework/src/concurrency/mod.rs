//! Running several tasks at once: the `Concurrency` facade.
//!
//! [`Concurrency::run`] runs tasks side by side and answers their results
//! in order; [`Concurrency::run_named`] answers them by key;
//! [`Concurrency::defer`] runs them after the response has been sent and
//! discards what they answer. A task is an `async fn` marked
//! `#[concurrent_task]`, registered at compile time under a name, with
//! serde input and output:
//!
//! ```rust,ignore
//! use suprnova::concurrency::{Concurrency, Task};
//! use suprnova::{FrameworkError, concurrent_task};
//!
//! #[concurrent_task]
//! async fn count_rows(table: String) -> Result<u64, FrameworkError> {
//!     // ...
//! #   Ok(0)
//! }
//!
//! let results = Concurrency::run([
//!     Task::new::<CountRows>("users".to_owned()),
//!     Task::new::<CountRows>("orders".to_owned()),
//! ])
//! .await?;
//! let users: u64 = results[0].get()?;
//! ```
//!
//! # Drivers
//!
//! A [`ConcurrencyDriver`] runs the tasks. `tokio`, the default, runs each
//! as a Tokio task; `process` runs each in a console process of its own
//! for the isolation a Tokio task lacks; `sync` awaits them one after
//! another, for tests. `CONCURRENCY_DRIVER` names the default,
//! [`Concurrency::driver`] picks one for a call, and
//! [`Concurrency::extend`] adds a driver of your own. Laravel's `fork`
//! driver has no counterpart: the process driver is the isolating one.
//!
//! Why tasks are named functions rather than closures: the process driver
//! sends each task to another process, and a Rust closure cannot cross a
//! process boundary. The name crosses it, and the console binary finds the
//! same function in its own registry.

mod console;
mod driver;
mod process;
mod sync;
mod task;
mod tokio;

pub use driver::{ConcurrencyDriver, timed_out};
pub use process::ProcessConcurrencyDriver;
pub use sync::SyncConcurrencyDriver;
pub use task::{
    ConcurrentTask, ConcurrentTaskEntry, Task, TaskFuture, TaskResult, find, registered,
};
pub use tokio::TokioConcurrencyDriver;

use crate::bus::after_response::{self, CommandFuture};
use crate::error::FrameworkError;
use indexmap::IndexMap;
use std::collections::HashSet;
use std::future::{Future, IntoFuture};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// The variable that names the default driver.
pub const DRIVER_ENV: &str = "CONCURRENCY_DRIVER";

/// The driver used when [`DRIVER_ENV`] is unset or empty.
pub const DEFAULT_DRIVER: &str = "tokio";

/// Drivers added with [`Concurrency::extend`], by name.
static EXTENDED: RwLock<Vec<(String, Arc<dyn ConcurrencyDriver>)>> = RwLock::new(Vec::new());

/// The facade. Every method either runs tasks on the default driver or
/// starts a [`PendingConcurrency`] that sets the driver or the timeout for
/// one call.
#[derive(Debug, Clone, Copy)]
pub struct Concurrency;

impl Concurrency {
    /// Run `tasks` side by side on the default driver and answer each
    /// one's result, in the order of `tasks`.
    ///
    /// # Errors
    ///
    /// The first failed task's error, in the order of `tasks`, once every
    /// task has ended; an error naming the driver when no driver has the
    /// default's name.
    pub async fn run<I>(tasks: I) -> Result<Vec<TaskResult>, FrameworkError>
    where
        I: IntoIterator<Item = Task>,
    {
        PendingConcurrency::default().run(tasks).await
    }

    /// Run keyed `tasks` side by side on the default driver and answer each
    /// one's result under its key, in the order of `tasks`.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run), and an error naming a key that two tasks
    /// share, before any task runs.
    pub async fn run_named<K, I>(tasks: I) -> Result<IndexMap<String, TaskResult>, FrameworkError>
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, Task)>,
    {
        PendingConcurrency::default().run_named(tasks).await
    }

    /// Run `tasks` on the default driver after the current response has
    /// been sent, and discard their results. See
    /// [`PendingConcurrency::defer`].
    ///
    /// # Errors
    ///
    /// An error naming the driver when no driver has the default's name.
    pub fn defer<I>(tasks: I) -> Result<DeferredTasks, FrameworkError>
    where
        I: IntoIterator<Item = Task>,
    {
        PendingConcurrency::default().defer(tasks)
    }

    /// Use the driver named `name` for one call, as Laravel's
    /// `Concurrency::driver('sync')->run(...)`.
    pub fn driver(name: impl Into<String>) -> PendingConcurrency {
        PendingConcurrency::default().driver(name)
    }

    /// End a task that runs longer than `timeout` with an error naming it,
    /// for one call. Zero means no timeout. Without one a task runs until
    /// it finishes, on every driver.
    pub fn with_timeout(timeout: Duration) -> PendingConcurrency {
        PendingConcurrency::default().with_timeout(timeout)
    }

    /// The name of the default driver: `CONCURRENCY_DRIVER`, read at each
    /// call, or `tokio` when it is unset or empty.
    pub fn default_driver() -> String {
        match std::env::var(DRIVER_ENV) {
            Ok(name) if !name.trim().is_empty() => name.trim().to_owned(),
            _ => DEFAULT_DRIVER.to_owned(),
        }
    }

    /// Add a driver under `name`, or replace the one that has it, as
    /// Laravel's `Concurrency::extend`. A driver added under `tokio`,
    /// `process` or `sync` takes the place of the built-in one.
    ///
    /// # Errors
    ///
    /// An error when the driver registry's lock is poisoned.
    pub fn extend(
        name: impl Into<String>,
        driver: impl ConcurrencyDriver,
    ) -> Result<(), FrameworkError> {
        let name = name.into();
        let driver: Arc<dyn ConcurrencyDriver> = Arc::new(driver);
        let mut extended = crate::lock::write(&EXTENDED, "concurrency drivers")?;
        match extended.iter_mut().find(|(held, _)| *held == name) {
            Some(slot) => slot.1 = driver,
            None => extended.push((name, driver)),
        }
        Ok(())
    }

    /// Start the process driver's tasks with the console binary at `path`.
    ///
    /// Without it the driver starts the `console` binary beside the running
    /// executable, as a Suprnova project builds the two side by side, or the
    /// running executable itself when there is none. Set it when the
    /// console binary lives elsewhere, as in a container image that keeps
    /// it in another directory.
    ///
    /// # Errors
    ///
    /// An error when `path` is not UTF-8, which the Process facade cannot
    /// start, or when the setting's lock is poisoned.
    pub fn console_binary(path: impl AsRef<Path>) -> Result<(), FrameworkError> {
        let program = process::path_text(path.as_ref().to_path_buf())?;
        process::set_console_command(vec![program])
    }

    /// Start the process driver's tasks with `command`: the program and the
    /// arguments that come before `concurrency:run <task>`.
    ///
    /// For a console that is started through another program, such as
    /// `["cargo", "run", "--quiet", "--bin", "console", "--"]` in
    /// development, or a wrapper that sets up the environment first.
    ///
    /// # Errors
    ///
    /// An error when `command` is empty, since it names no program, or when
    /// the setting's lock is poisoned.
    pub fn console_command<I, S>(command: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let command: Vec<String> = command.into_iter().map(Into::into).collect();
        if command.is_empty() {
            return Err(FrameworkError::internal(
                "Concurrency::console_command needs a program to start; the list is empty",
            ));
        }
        process::set_console_command(command)
    }
}

/// One call to the facade with its driver or its timeout set:
/// `Concurrency::driver("process").with_timeout(...).run(...)`.
#[derive(Debug, Clone, Default)]
pub struct PendingConcurrency {
    driver: Option<String>,
    timeout: Option<Duration>,
}

impl PendingConcurrency {
    /// Use the driver named `name` instead of the default.
    pub fn driver(mut self, name: impl Into<String>) -> Self {
        self.driver = Some(name.into());
        self
    }

    /// End a task that runs longer than `timeout` with an error naming it.
    /// Zero means no timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = (!timeout.is_zero()).then_some(timeout);
        self
    }

    /// The name of the driver this call uses.
    pub fn driver_name(&self) -> String {
        self.driver
            .clone()
            .unwrap_or_else(Concurrency::default_driver)
    }

    /// Run `tasks` side by side and answer each one's result, in the order
    /// of `tasks`. Every task runs to its end, or to the timeout, before the
    /// call answers, even when one fails.
    ///
    /// # Errors
    ///
    /// The first failed task's error, in the order of `tasks`; an error
    /// naming the driver when no driver has its name.
    pub async fn run<I>(self, tasks: I) -> Result<Vec<TaskResult>, FrameworkError>
    where
        I: IntoIterator<Item = Task>,
    {
        let (name, driver) = self.resolve()?;
        let tasks: Vec<Task> = tasks.into_iter().collect();
        run_on(&name, driver.as_ref(), tasks, self.timeout)
            .await?
            .into_iter()
            .collect()
    }

    /// Run keyed `tasks` side by side and answer each one's result under its
    /// key, in the order of `tasks`, as Laravel's `run` answers an
    /// associative array.
    ///
    /// # Errors
    ///
    /// As [`run`](Self::run), and an error naming a key that two tasks
    /// share, before any task runs.
    pub async fn run_named<K, I>(
        self,
        tasks: I,
    ) -> Result<IndexMap<String, TaskResult>, FrameworkError>
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, Task)>,
    {
        let mut keys = Vec::new();
        let mut list = Vec::new();
        let mut seen = HashSet::new();
        for (key, task) in tasks {
            let key = key.into();
            if !seen.insert(key.clone()) {
                return Err(FrameworkError::internal(format!(
                    "Concurrency::run_named: the key `{key}` names two tasks"
                )));
            }
            keys.push(key);
            list.push(task);
        }
        let results = self.run(list).await?;
        Ok(keys.into_iter().zip(results).collect())
    }

    /// Run `tasks` on this call's driver after the current response has
    /// been sent, as Laravel's `Concurrency::defer`, and discard their
    /// results.
    ///
    /// Inside a request the call returns at once. The tasks start once the
    /// response's last bytes have been written and flushed, through the
    /// same facility as `Bus::dispatch_after_response`. A task that fails
    /// is logged, with its name and error, and the response, already sent,
    /// is not touched.
    ///
    /// Outside a request, as in a console command or a worker, there is no
    /// response to wait for: the returned [`DeferredTasks`] runs the tasks
    /// when it is awaited, or starts them on the runtime when it is dropped.
    /// The same holds where no response writer is in reach, as in a custom
    /// Hyper loop that does not use `after_response_connection` or the
    /// in-memory test client.
    ///
    /// # Errors
    ///
    /// An error naming the driver when no driver has its name. A task's
    /// failure is never an error of this call.
    pub fn defer<I>(self, tasks: I) -> Result<DeferredTasks, FrameworkError>
    where
        I: IntoIterator<Item = Task>,
    {
        let (name, driver) = self.resolve()?;
        let tasks: Vec<Task> = tasks.into_iter().collect();
        let timeout = self.timeout;
        let work: CommandFuture = Box::pin(crate::App::in_current_scope(async move {
            run_deferred(&name, driver.as_ref(), tasks, timeout).await;
            Ok(())
        }));
        Ok(match after_response::try_defer(work) {
            Ok(()) => DeferredTasks { pending: None },
            Err(work) => DeferredTasks {
                pending: Some(work),
            },
        })
    }

    /// The driver's name and the driver.
    fn resolve(&self) -> Result<(String, Arc<dyn ConcurrencyDriver>), FrameworkError> {
        let name = self.driver_name();
        let driver = driver_named(&name)?;
        Ok((name, driver))
    }
}

/// The driver named `name`: one added with [`Concurrency::extend`], or a
/// built-in one.
fn driver_named(name: &str) -> Result<Arc<dyn ConcurrencyDriver>, FrameworkError> {
    if let Some(driver) = crate::lock::read(&EXTENDED, "concurrency drivers")?
        .iter()
        .find(|(held, _)| held == name)
        .map(|(_, driver)| Arc::clone(driver))
    {
        return Ok(driver);
    }
    match name {
        "tokio" => Ok(Arc::new(TokioConcurrencyDriver)),
        "process" => Ok(Arc::new(ProcessConcurrencyDriver)),
        "sync" => Ok(Arc::new(SyncConcurrencyDriver)),
        other => Err(FrameworkError::internal(format!(
            "no concurrency driver is named `{other}`; the drivers are tokio, process and \
             sync, and Concurrency::extend adds others"
        ))),
    }
}

/// Run the tasks on the driver and check it answered one outcome per task.
async fn run_on(
    name: &str,
    driver: &dyn ConcurrencyDriver,
    tasks: Vec<Task>,
    timeout: Option<Duration>,
) -> Result<Vec<Result<TaskResult, FrameworkError>>, FrameworkError> {
    let expected = tasks.len();
    if expected == 0 {
        return Ok(Vec::new());
    }
    let outcomes = driver.run(tasks, timeout).await;
    if outcomes.len() != expected {
        return Err(FrameworkError::internal(format!(
            "the concurrency driver `{name}` answered {} outcomes for {expected} tasks",
            outcomes.len()
        )));
    }
    Ok(outcomes)
}

/// Run deferred tasks and log every failure, since no caller is left to
/// answer.
async fn run_deferred(
    driver_name: &str,
    driver: &dyn ConcurrencyDriver,
    tasks: Vec<Task>,
    timeout: Option<Duration>,
) {
    let names: Vec<String> = tasks.iter().map(|task| task.name().to_owned()).collect();
    match run_on(driver_name, driver, tasks, timeout).await {
        Ok(outcomes) => {
            for (task, outcome) in names.iter().zip(outcomes) {
                if let Err(error) = outcome {
                    tracing::error!(
                        task = %task,
                        driver = %driver_name,
                        %error,
                        "deferred concurrent task failed"
                    );
                }
            }
        }
        Err(error) => tracing::error!(
            driver = %driver_name,
            %error,
            "deferred concurrent tasks failed"
        ),
    }
}

/// Deferred tasks, as [`Concurrency::defer`] returns them.
///
/// Inside a request the tasks already wait for the response, and this
/// holds nothing. Outside one it holds the tasks: awaiting it runs them and
/// waits for them, dropping it starts them on the Tokio runtime and returns.
/// A process that ends right after the drop can end before the tasks do, so
/// await it where the process may end, as at the end of a console command.
pub struct DeferredTasks {
    pending: Option<CommandFuture>,
}

impl DeferredTasks {
    /// Whether the tasks wait for the current response: `true` inside a
    /// request served with a response writer, `false` when this handle
    /// runs them.
    pub fn after_response(&self) -> bool {
        self.pending.is_none()
    }
}

impl std::fmt::Debug for DeferredTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeferredTasks")
            .field("after_response", &self.after_response())
            .finish()
    }
}

impl IntoFuture for DeferredTasks {
    type Output = ();
    type IntoFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

    fn into_future(mut self) -> Self::IntoFuture {
        let pending = self.pending.take();
        Box::pin(async move {
            if let Some(work) = pending {
                // The work logs its own failures and answers `Ok`.
                let _logged = work.await;
            }
        })
    }
}

impl Drop for DeferredTasks {
    fn drop(&mut self) {
        let Some(work) = self.pending.take() else {
            return;
        };
        match ::tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                // Detached: the tasks run on, and log their own failures.
                drop(runtime.spawn(work));
            }
            // No runtime to start them on, as in a plain `main`: run them
            // here, on a runtime of their own, before the drop returns.
            Err(_) => match ::tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => {
                    let _logged = runtime.block_on(work);
                }
                Err(error) => tracing::error!(
                    %error,
                    "deferred concurrent tasks did not run: no Tokio runtime could be built"
                ),
            },
        }
    }
}
