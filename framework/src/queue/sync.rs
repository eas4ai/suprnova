//! Synchronous queue driver - runs jobs inline on `push`.
//!
//! Mirrors Laravel's `SyncQueue`. The envelope runs inline through the
//! worker's middleware pipeline (`run_through_middleware`) before `push`
//! returns, and so does every later link of a chain, until one does not
//! complete. A batch job is settled in its batch as a worker would settle it,
//! so a batch on this driver finishes and fires its callbacks. There is no
//! background worker, no retry, and no delayed-job support - `push` for an
//! envelope with `available_at` in the future runs immediately anyway, just
//! like Laravel's sync driver (a "fake" queue for development).
//!
//! Use this driver in stages where the queue infra isn't desired (CI without
//! Redis, local dev), or in tests that need handlers to actually execute
//! without the worker loop. Production deployments should use Memory (for
//! single-process apps), Redis, or Database.

use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken};
use crate::queue::envelope::Envelope;
use crate::queue::inspect::InspectedJob;
use crate::queue::outcome::JobOutcome;
use crate::queue::worker::{chain_successor, run_through_middleware};
use async_trait::async_trait;
use std::time::Duration;

/// [`QueueDriver`] that runs each pushed job inline on the calling
/// task. Mirrors Laravel's `sync` driver - useful in tests and in
/// configurations where no background worker is desired.
#[derive(Default)]
pub struct SyncQueueDriver;

impl SyncQueueDriver {
    /// Construct a fresh sync driver.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl QueueDriver for SyncQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        // Sync drivers run the job inline through the SAME middleware
        // pipeline the worker uses (`run_through_middleware`), so
        // `WithoutOverlapping`, `RateLimited`, `ThrottlesExceptions`, etc.
        // apply here exactly as they would on a background worker - running
        // the bare dispatcher would silently bypass every registered
        // `JobMiddleware`. If no dispatcher is registered for this job_name,
        // the pipeline's terminal returns the same "unknown job" error a
        // worker would. Errors propagate to the caller of `Queue::push`;
        // there is no retry or loop-lifecycle event path because there is no
        // background worker (the loop owns reservation/retry/event state).
        //
        // A chain continues inline, link by link, as Laravel's sync queue
        // dispatches the next job of a chain from the one that just ran. As
        // on a worker, only a completed link starts the next one: an error
        // returns here with the rest of the chain unrun, and a link that
        // middleware released, failed or deleted ends the chain. The queue
        // fake decides for each later link, as it does on a worker.
        //
        // Each job that belongs to a batch is settled in it as a worker would
        // settle it: a completed or deleted job as a success, a failed one as
        // a failure. A released job is left pending, as Laravel's sync queue
        // leaves it: a worker would run it again later, and nothing runs it
        // here.
        let mut current = Some(env);
        while let Some(env) = current.take() {
            let outcome = match run_through_middleware(env.clone()).await {
                Ok(outcome) => outcome,
                Err(e) => {
                    settle_batch_job(&env, false).await;
                    return Err(e);
                }
            };
            match outcome {
                JobOutcome::Completed | JobOutcome::Deleted => settle_batch_job(&env, true).await,
                JobOutcome::Failed { .. } => settle_batch_job(&env, false).await,
                JobOutcome::Released { .. } => {}
            }
            if !matches!(outcome, JobOutcome::Completed) {
                break;
            }
            let Some(next) = chain_successor(&env, &crate::queue::Queue::connection_name()) else {
                break;
            };
            if crate::queue::testing::fakes(&next.job_name) {
                crate::queue::testing::record_envelope(&next);
                break;
            }
            current = Some(next);
        }
        Ok(())
    }

    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        // Sync driver has no queue to pop from; the worker should never
        // be running against it.
        Ok(None)
    }

    async fn ack(&self, _t: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(&self, _t: &ReservationToken, _delay: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    /// Always empty: the sync driver runs every job inline on `push`, so
    /// nothing is ever left pending. `Ok(vec![])` is the honest answer here
    /// - not a lie of omission the way Laravel's Beanstalkd/SQS stubs are -
    /// because for this driver "nothing to list" is the literal truth, not
    /// an unimplemented method. See the trait default's doc comment on
    /// [`QueueDriver::pending_jobs`].
    async fn pending_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    /// Always empty: the sync driver has no delayed-job support at all -
    /// `push` runs immediately even for an envelope with a future
    /// `available_at`. See [`pending_jobs`](Self::pending_jobs).
    async fn delayed_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    /// Always empty: there is no background worker to hold a reservation.
    /// See [`pending_jobs`](Self::pending_jobs).
    async fn reserved_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    fn name(&self) -> &'static str {
        "sync"
    }
}

/// Settle `env` in its batch, logging a bookkeeping failure rather than
/// returning it: the job has already run, and an `Err` from `push` would tell
/// the caller it had not.
async fn settle_batch_job(env: &Envelope, succeeded: bool) {
    if let Err(e) = crate::queue::batch::settle_inline(env, succeeded).await {
        tracing::warn!(
            job = %env.job_name,
            id = %env.id,
            batch_id = ?env.batch_id,
            error = %e,
            "sync queue: could not settle an inline batch job in its batch"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Job;
    use crate::queue::worker::register_job;
    use crate::queue::{CURRENT_SCHEMA_VERSION, Envelope};
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};
    use serial_test::serial;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use uuid::Uuid;

    static SYNC_RUNS: AtomicU32 = AtomicU32::new(0);

    #[derive(Serialize, Deserialize)]
    struct SyncJob {
        x: i32,
    }

    #[async_trait]
    impl Job for SyncJob {
        fn job_name() -> &'static str {
            "queue::sync::tests::SyncJob"
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            SYNC_RUNS.fetch_add(self.x as u32, Ordering::SeqCst);
            Ok(())
        }
    }

    fn env_for<J: Job>(job: &J) -> Envelope {
        Envelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            job_name: J::job_name().into(),
            queue: None,
            payload: serde_json::to_value(job).unwrap(),
            dispatched_at: crate::clock::now(),
            available_at: crate::clock::now(),
            attempts: 0,
            max_tries: 1,
            backoff: crate::queue::BackoffSchedule::default(),
            timeout_secs: None,
            fail_on_timeout: false,
            idempotency_key: None,
            unique_lock_owner: None,
            debounce_id: None,
            debounce_owner: None,
            batch_id: None,
            chain_remaining: Vec::new(),
            context: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn sync_driver_runs_inline() {
        SYNC_RUNS.store(0, Ordering::SeqCst);
        register_job::<SyncJob>();
        let d = Arc::new(SyncQueueDriver::new());
        d.push(env_for(&SyncJob { x: 7 })).await.unwrap();
        assert_eq!(SYNC_RUNS.load(Ordering::SeqCst), 7);
    }
}
