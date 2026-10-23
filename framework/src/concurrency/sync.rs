//! The `sync` driver: the tasks one after another, in the caller's task.

use super::driver::{ConcurrencyDriver, timed_out};
use super::task::{Task, TaskResult};
use crate::error::FrameworkError;
use async_trait::async_trait;
use std::time::Duration;

/// Awaits the tasks one after another in the caller's own task, as
/// Laravel's `sync` driver calls its closures in sequence.
///
/// For tests that want concurrency out of the way: the tasks run in a known
/// order, each sees what the one before it did, and nothing outlives the
/// call. The input and output still pass through JSON, as on the other
/// drivers. A timeout applies to each task from its own start.
#[derive(Debug, Clone, Copy, Default)]
pub struct SyncConcurrencyDriver;

#[async_trait]
impl ConcurrencyDriver for SyncConcurrencyDriver {
    async fn run(
        &self,
        tasks: Vec<Task>,
        timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        let mut outcomes = Vec::with_capacity(tasks.len());
        for task in tasks {
            let name = task.name().to_owned();
            let outcome = match timeout {
                Some(timeout) => tokio::time::timeout(timeout, task.run_in_process())
                    .await
                    .unwrap_or_else(|_elapsed| Err(timed_out(&name, timeout))),
                None => task.run_in_process().await,
            };
            outcomes.push(outcome);
        }
        outcomes
    }
}
