//! The trait every concurrency driver implements.

use super::task::{Task, TaskResult};
use crate::error::FrameworkError;
use async_trait::async_trait;
use std::time::Duration;

/// A way to run several tasks at once: on the Tokio runtime, in console
/// processes, one after another, or anything else that can run a
/// registered task by name.
///
/// The facade picks a driver by name (`CONCURRENCY_DRIVER`,
/// [`Concurrency::driver`](super::Concurrency::driver)) and hands it the
/// tasks. A driver answers one outcome per task, in the order it was given
/// the tasks; the facade turns them into the caller's results, or logs them
/// for deferred tasks. Deferral is the facade's: it runs the driver after
/// the response, so a driver implements only `run`.
///
/// Register a driver of your own with
/// [`Concurrency::extend`](super::Concurrency::extend).
#[async_trait]
pub trait ConcurrencyDriver: Send + Sync + 'static {
    /// Run every task and answer each one's outcome, in the order of
    /// `tasks`.
    ///
    /// With a `timeout`, a task still running when it passes is ended and
    /// its outcome is an error naming it; [`timed_out`] builds that error.
    /// One task's failure does not stop the others.
    async fn run(
        &self,
        tasks: Vec<Task>,
        timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>>;
}

/// The error for a task ended by its timeout: a
/// [`FrameworkError::Timeout`] carrying the timeout and naming the task, so
/// [`FrameworkError::is_timeout`] tells it from a failure of the task.
pub fn timed_out(task: &str, timeout: Duration) -> FrameworkError {
    FrameworkError::timeout(
        timeout,
        format!("concurrent task `{task}` was ended by its timeout"),
    )
}
