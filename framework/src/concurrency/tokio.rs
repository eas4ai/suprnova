//! The `tokio` driver: each task a Tokio task of its own, the default.

use super::driver::{ConcurrencyDriver, timed_out};
use super::task::{Task, TaskResult};
use crate::error::FrameworkError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;
use tokio::task::JoinSet;

/// Runs each task as a Tokio task of its own, so the tasks run at once on
/// the runtime's threads, and joins them all.
///
/// It is the cheapest driver and the default. The tasks share the process
/// with the caller: a task that blocks a thread or panics is in the same
/// address space, and a timeout ends a task only at its next `.await`.
/// Use the process driver when a task needs isolation.
///
/// Each task runs in the caller's container scope, so a scoped binding the
/// request resolved is the one the task sees.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioConcurrencyDriver;

#[async_trait]
impl ConcurrencyDriver for TokioConcurrencyDriver {
    async fn run(
        &self,
        tasks: Vec<Task>,
        timeout: Option<Duration>,
    ) -> Vec<Result<TaskResult, FrameworkError>> {
        let names: Vec<String> = tasks.iter().map(|task| task.name().to_owned()).collect();
        let mut outcomes: Vec<Option<Result<TaskResult, FrameworkError>>> = vec![None; names.len()];
        // A set aborts what it still holds when it is dropped: a caller
        // that stops waiting, or a timeout, leaves no task running.
        let mut set = JoinSet::new();
        let mut index_of = HashMap::with_capacity(names.len());
        for (index, task) in tasks.into_iter().enumerate() {
            let handle = set.spawn(crate::App::in_current_scope(task.run_in_process()));
            index_of.insert(handle.id(), index);
        }
        // Every task starts now, so one deadline is each task's timeout.
        let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
        loop {
            let joined = match deadline {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, set.join_next_with_id()).await {
                        Ok(joined) => joined,
                        Err(_elapsed) => break,
                    }
                }
                None => set.join_next_with_id().await,
            };
            let Some(joined) = joined else { break };
            let (id, outcome) = match joined {
                Ok((id, outcome)) => (id, outcome),
                // The task catches its own panic, so this is a task that was
                // cancelled from outside, such as by a runtime shutting down.
                Err(error) => {
                    let id = error.id();
                    let name = index_of
                        .get(&id)
                        .and_then(|index| names.get(*index))
                        .map_or("", String::as_str);
                    (
                        id,
                        Err(FrameworkError::internal(format!(
                            "concurrent task `{name}` did not finish: {error}"
                        ))),
                    )
                }
            };
            if let Some(slot) = index_of.get(&id).and_then(|index| outcomes.get_mut(*index)) {
                *slot = Some(outcome);
            }
        }
        set.abort_all();
        outcomes
            .into_iter()
            .zip(names)
            .map(|(outcome, name)| {
                outcome.unwrap_or_else(|| match timeout {
                    Some(timeout) => Err(timed_out(&name, timeout)),
                    None => Err(FrameworkError::internal(format!(
                        "concurrent task `{name}` did not finish"
                    ))),
                })
            })
            .collect()
    }
}
