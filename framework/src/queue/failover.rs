//! Failover queue connection - an ordered list of driver connections where a
//! push that one connection refuses falls through to the next.
//!
//! Mirrors Laravel 13's `Illuminate\Queue\FailoverQueue`, including the
//! per-job `bulk` loop that PR #60950 added so a batch does not lose each
//! job's own delay on the way through.

use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken, Settled};
use crate::queue::envelope::Envelope;
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Wraps an ordered list of queue connections: a push that the first
/// connection refuses is retried on the next, and so on down the list.
///
/// # Writes fall through, reads do not
///
/// Only [`push`](QueueDriver::push) and [`bulk_push`](QueueDriver::bulk_push)
/// walk the list. Everything else - `pop`, `pop_from`, `ack`, `nack`,
/// `release`, `settle`, `clear`, all four counters and all three listings -
/// delegates to the first connection only. This is not laziness: reservation
/// tokens are meaningful only to the driver that issued them, so routing
/// lifecycle calls anywhere else would corrupt state. The counters and
/// listings follow the same rule so that what an operator inspects is the
/// same backend the worker is draining, rather than a sum across connections
/// that matches no single worker's view.
///
/// The operational consequence is the one Laravel's own docs carry: a worker
/// pointed at the failover connection drains the **primary** only. Jobs that
/// failed over to a fallback need a worker running against that fallback
/// connection directly, or they sit there.
///
/// # What `bulk_push` guarantees
///
/// `bulk_push` loops [`push`](QueueDriver::push) per envelope rather than
/// forwarding the whole batch to an inner driver's `bulk_push`. Two reasons,
/// and both matter:
///
/// - Each envelope already carries its own `available_at`, resolved before
///   the driver ever sees it. Pushing them one at a time is what preserves
///   that; a wholesale re-push would be free to flatten it (Laravel #60950).
/// - A batch that half-lands on connection A before A dies would be
///   double-pushed on B if the remainder were retried wholesale. Per-envelope
///   fall-through gives at-most-once *per envelope* across backends, which is
///   the strongest claim available here - the framework cannot promise
///   at-most-once for the batch, because an envelope A accepted and then
///   failed to acknowledge is indistinguishable from one it never took.
///
/// A `bulk_push` where one envelope is refused by every connection returns
/// that envelope's error with the earlier envelopes already enqueued, exactly
/// as the trait's own serial default does.
///
/// # Events
///
/// Each connection that refuses a push dispatches
/// [`QueueFailedOver`](crate::queue::events::QueueFailedOver) - but only on
/// the push that moves it *into* failure. A connection that is already known
/// to be failing stays quiet until a later push succeeds on it, which re-arms
/// it. That keeps a multi-hour outage to one alert instead of one per
/// dispatch.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use suprnova::queue::{FailoverQueueDriver, MemoryQueueDriver, Queue, QueueDriver};
/// # fn main() -> Result<(), suprnova::FrameworkError> {
/// let failover = FailoverQueueDriver::new(vec![
///     ("redis".to_string(), Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>),
///     ("database".to_string(), Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>),
/// ])?;
/// Queue::set_driver(Arc::new(failover));
/// # Ok(())
/// # }
/// ```
pub struct FailoverQueueDriver {
    /// Ordered connections, primary first. The `String` is the configured
    /// label reported on `QueueFailedOver`; [`QueueDriver::name`] cannot
    /// stand in for it because it names the driver *type* (two `redis`
    /// connections would be indistinguishable).
    drivers: Vec<(String, Arc<dyn QueueDriver>)>,
    /// The same `Arc` as the first entry of `drivers`, held separately so
    /// every read-side delegation is a field access instead of an index into
    /// a `Vec` whose non-emptiness is only a constructor invariant. Cheap:
    /// an `Arc` clone taken once, at construction.
    primary: Arc<dyn QueueDriver>,
    /// Indices into `drivers` that were failing as of the last push attempt.
    /// Replaced wholesale after each attempt so a success clears it and
    /// re-arms the event.
    ///
    /// This gates event emission only - it never gates routing, so two
    /// concurrent pushes that both observe the same pre-transition snapshot
    /// can at worst emit one duplicate `QueueFailedOver`. No job is ever
    /// routed differently because of what this set says.
    failing: Mutex<HashSet<usize>>,
}

impl FailoverQueueDriver {
    /// Build a failover connection over `drivers`, in priority order.
    ///
    /// Each `String` is the connection label carried on
    /// [`QueueFailedOver`](crate::queue::events::QueueFailedOver) - use the
    /// configured connection name (`"redis"`, `"database"`), not the driver
    /// type.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::internal`] when `drivers` is empty. A
    /// failover connection with nothing to fail over to would accept
    /// `Queue::set_driver` and then fail every push at runtime; rejecting it
    /// here turns a silent outage into a boot error.
    pub fn new(drivers: Vec<(String, Arc<dyn QueueDriver>)>) -> Result<Self, FrameworkError> {
        let primary = drivers.first().map(|(_, d)| Arc::clone(d)).ok_or_else(|| {
            FrameworkError::internal(
                "FailoverQueueDriver requires at least one connection; \
                 an empty list has nothing to push to",
            )
        })?;
        Ok(Self {
            drivers,
            primary,
            failing: Mutex::new(HashSet::new()),
        })
    }

    /// Snapshot of the connections that were failing as of the last attempt.
    ///
    /// A poisoned mutex is recovered rather than propagated: this set decides
    /// whether an event fires, and losing the queue because event bookkeeping
    /// panicked somewhere would be a strictly worse trade.
    fn previously_failing(&self) -> HashSet<usize> {
        self.failing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replace the failing set wholesale, per Laravel's `finally` block.
    fn record_failing(&self, failed: HashSet<usize>) {
        *self
            .failing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = failed;
    }

    /// Try `env` on each connection in order, returning on the first success.
    ///
    /// Ports Laravel's `attemptOnAllConnections`: fresh per-attempt failure
    /// set, edge-triggered events against the previous one, wholesale
    /// replacement afterwards, last error on total failure.
    async fn push_attempting_all(&self, env: Envelope) -> Result<(), FrameworkError> {
        let previously_failing = self.previously_failing();
        let mut failed = HashSet::new();
        let mut last_err = None;

        for (idx, (label, driver)) in self.drivers.iter().enumerate() {
            // Cloned per attempt because a refused push consumes the
            // envelope it was handed; the cost is a JSON payload clone
            // against a backend write, which is not where the time goes.
            match driver.push(env.clone()).await {
                Ok(()) => {
                    self.record_failing(failed);
                    return Ok(());
                }
                Err(e) => {
                    if !previously_failing.contains(&idx) {
                        // Best-effort, like every other queue event site: a
                        // listener that fails must not fail the push that is
                        // still trying to find a home for this job.
                        let _ = crate::events::EventFacade::dispatch(
                            crate::queue::events::QueueFailedOver {
                                connection: label.clone(),
                                job_name: env.job_name.clone(),
                                exception: e.to_string(),
                            },
                        )
                        .await;
                    }
                    // `remaining_connections` rather than "falling over to the
                    // next connection", because on the last entry there is no
                    // next one and the push is about to fail outright.
                    tracing::warn!(
                        connection = %label,
                        job = %env.job_name,
                        error = %e,
                        remaining_connections = self.drivers.len() - idx - 1,
                        "queue connection refused a push"
                    );
                    failed.insert(idx);
                    last_err = Some(e);
                }
            }
        }

        self.record_failing(failed);
        Err(last_err.unwrap_or_else(|| {
            // Unreachable: `new` rejects an empty list, so the loop above ran
            // at least once and either returned or recorded an error. Kept as
            // an error rather than an `expect` so a future constructor change
            // cannot turn this into a panic on the push path.
            FrameworkError::internal(
                "failover queue has no connections configured; nothing accepted the push",
            )
        }))
    }
}

#[async_trait]
impl QueueDriver for FailoverQueueDriver {
    // ---- write path: falls through the connection list -------------------

    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        self.push_attempting_all(env).await
    }

    async fn bulk_push(&self, envs: Vec<Envelope>) -> Result<(), FrameworkError> {
        for env in envs {
            self.push_attempting_all(env).await?;
        }
        Ok(())
    }

    // ---- read and lifecycle path: primary connection only ----------------
    //
    // Every method below delegates to `self.primary` and never consults a
    // fallback. For `pop` / `pop_from` / `ack` / `nack` / `release` /
    // `settle` that is a correctness requirement (a reservation token only
    // means something to the driver that issued it). For `clear`, the four
    // counters and the three inspection listings it is a consistency
    // requirement: an operator reading `pending_jobs` sees exactly what the
    // worker on this connection will drain, not a union across backends that
    // no worker actually consumes.

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.primary.pop(visibility_timeout).await
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.primary.pop_from(visibility_timeout, queues).await
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        self.primary.ack(token).await
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.primary.nack(token, requeue_delay).await
    }

    async fn release(
        &self,
        token: &ReservationToken,
        env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.primary.release(token, env, delay).await
    }

    async fn settle(
        &self,
        token: &ReservationToken,
        follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        self.primary.settle(token, follow_ups).await
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        self.primary.size().await
    }

    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        self.primary.pending_size().await
    }

    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        self.primary.delayed_size().await
    }

    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        self.primary.reserved_size().await
    }

    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.primary.pending_jobs(queue).await
    }

    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.primary.delayed_jobs(queue).await
    }

    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.primary.reserved_jobs(queue).await
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        self.primary.clear().await
    }

    fn name(&self) -> &'static str {
        "failover"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::memory::MemoryQueueDriver;

    #[test]
    fn new_rejects_an_empty_connection_list() {
        // `.err().expect(...)` rather than `expect_err`: no queue driver in
        // this module implements `Debug`, and adding one just to phrase an
        // assertion would be the tail wagging the dog.
        let err = FailoverQueueDriver::new(vec![])
            .err()
            .expect("empty list must be rejected");
        assert!(
            err.to_string().contains("at least one connection"),
            "the error must name the misconfiguration, got {err}"
        );
    }

    // `MemoryQueueDriver::new` builds a `DelayQueue`, which needs a reactor.
    #[tokio::test]
    async fn new_holds_the_first_connection_as_the_primary() {
        let driver = FailoverQueueDriver::new(vec![
            (
                "one".to_string(),
                Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>,
            ),
            (
                "two".to_string(),
                Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>,
            ),
        ])
        .expect("two connections");
        assert!(
            Arc::ptr_eq(&driver.primary, &driver.drivers[0].1),
            "the cached primary must be the first configured connection"
        );
        assert_eq!(driver.name(), "failover");
    }
}
