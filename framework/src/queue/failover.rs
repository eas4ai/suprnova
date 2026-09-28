//! Failover queue connection - an ordered list of driver connections where a
//! push that one connection refuses falls through to the next and workers
//! drain every connection that accepted work.
//!
//! Mirrors Laravel 13's `Illuminate\Queue\FailoverQueue`, including the
//! per-job `bulk` loop that PR #60950 added so a batch does not lose each
//! job's own delay on the way through.

use crate::error::FrameworkError;
use crate::queue::driver::{
    QueueDriver, QueueFilterCapability, Reservation, ReservationToken, Settled,
};
use crate::queue::envelope::Envelope;
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

struct Connection {
    label: String,
    driver: Arc<dyn QueueDriver>,
    gate: Arc<RwLock<()>>,
}

#[derive(Clone)]
struct ReservationOrigin {
    connection: Arc<Connection>,
    inner_token: ReservationToken,
    lease_deadline: Instant,
}

/// Wraps an ordered list of queue connections: a push that the first
/// connection refuses is retried on the next, and so on down the list.
///
/// # Reads and reservation ownership
///
/// [`pop`](QueueDriver::pop) and [`pop_from`](QueueDriver::pop_from) rotate
/// their starting connection, then scan the full list sequentially. Rotation
/// prevents a recovered, continuously busy primary from starving work that
/// landed on a fallback. Polling stays sequential so one call cannot reserve
/// several jobs and return only one of them.
///
/// Each returned reservation receives a fresh aggregate token. The driver
/// retains a short-lived mapping from that token to the issuing connection's
/// real token, then routes `ack`, `nack`, `release`, and `settle` back to that
/// exact connection. This indirection is required because inner tokens are
/// not globally unique: two backends may legitimately issue the same UUID.
/// Expired or unknown aggregate tokens are treated as stale and never sent to
/// an arbitrary connection.
///
/// Counters and listings aggregate every configured connection in configured
/// order, and `clear` attempts every connection. The observable backlog thus
/// matches the work this aggregate driver can consume.
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
/// - The decorator never re-attempts an envelope a connection **accepted**.
///   A batch that half-landed on connection A is not re-pushed onto B when A
///   then dies mid-batch; only the envelopes A actually refused fall through.
///   Forwarding the batch wholesale would re-push the accepted ones too.
///
/// That second point is a bound on *re-attempts*, not a delivery guarantee,
/// and the difference matters: a connection that writes the envelope and
/// *then* reports failure still yields a duplicate on the next connection,
/// because "wrote it and lost the acknowledgement" and "never took it" are
/// indistinguishable from here. The envelope keeps its id, so both copies are
/// the same job. That is the framework's at-least-once delivery contract, not
/// a gap in this decorator - see the [worker module docs](crate::queue::worker):
/// every production handler must be idempotent.
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
    connections: Vec<Arc<Connection>>,
    /// Starting index for the next read. Incrementing once per aggregate poll
    /// distributes first choice across all connections without reserving from
    /// them concurrently.
    next_read: AtomicUsize,
    /// Aggregate reservation token to issuing driver and inner token.
    /// Entries expire with the visibility lease so a late worker cannot act
    /// on a token that a backend may already have reused after redelivery.
    reservations: Mutex<HashMap<ReservationToken, ReservationOrigin>>,
    /// Indices into `connections` currently believed to be failing.
    ///
    /// Written twice per push attempt, and the two writes do different jobs.
    /// An index is inserted the instant that connection refuses, as one
    /// check-and-set under the lock: the insert's own "was it absent?" answer
    /// is what decides whether the event fires, so two concurrent pushes
    /// cannot both find the connection healthy and both announce it. Then the
    /// whole set is replaced with the attempt's own failures when the attempt
    /// ends, which is what lets a success clear it and re-arm the event.
    ///
    /// This gates event emission only - it never gates routing. No job is ever
    /// placed differently because of what this set says.
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
        if drivers.is_empty() {
            return Err(FrameworkError::internal(
                "FailoverQueueDriver requires at least one connection; \
                 an empty list has nothing to push to",
            ));
        }
        let mut connections: Vec<Arc<Connection>> = Vec::with_capacity(drivers.len());
        for (label, driver) in drivers {
            let gate = connections
                .iter()
                .find(|connection| Arc::ptr_eq(&connection.driver, &driver))
                .map(|connection| Arc::clone(&connection.gate))
                .unwrap_or_else(|| Arc::new(RwLock::new(())));
            connections.push(Arc::new(Connection {
                label,
                driver,
                gate,
            }));
        }

        Ok(Self {
            connections,
            next_read: AtomicUsize::new(0),
            reservations: Mutex::new(HashMap::new()),
            failing: Mutex::new(HashSet::new()),
        })
    }

    fn remember_reservation(
        &self,
        connection: Arc<Connection>,
        reservation: Reservation,
        lease_deadline: Instant,
    ) -> Reservation {
        let now = Instant::now();
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reservations.retain(|_, origin| origin.lease_deadline > now);

        let aggregate_token = loop {
            let candidate = ReservationToken(Uuid::new_v4());
            if !reservations.contains_key(&candidate) {
                break candidate;
            }
        };
        reservations.insert(
            aggregate_token.clone(),
            ReservationOrigin {
                connection,
                inner_token: reservation.token,
                lease_deadline,
            },
        );
        Reservation {
            envelope: reservation.envelope,
            token: aggregate_token,
        }
    }

    fn reservation_origin(&self, token: &ReservationToken) -> Option<ReservationOrigin> {
        let now = Instant::now();
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reservations.retain(|_, origin| origin.lease_deadline > now);
        reservations.get(token).cloned()
    }

    async fn guarded_reservation_origin(
        &self,
        token: &ReservationToken,
    ) -> Option<(ReservationOrigin, tokio::sync::OwnedRwLockReadGuard<()>)> {
        let initial = self.reservation_origin(token)?;
        let connection = Arc::clone(&initial.connection);
        let gate = Arc::clone(&connection.gate).read_owned().await;
        let current = self.reservation_origin(token)?;
        if !Arc::ptr_eq(&connection, &current.connection) {
            return None;
        }
        Some((current, gate))
    }

    fn forget_reservation(&self, token: &ReservationToken) {
        self.reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(token);
    }

    fn forget_connection_reservations(&self, connection: &Connection) {
        self.reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, origin| !Arc::ptr_eq(&origin.connection.driver, &connection.driver));
    }

    fn prune_expired_reservations(&self) {
        let now = Instant::now();
        self.reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, origin| origin.lease_deadline > now);
    }

    async fn pop_rotating(
        &self,
        visibility_timeout: Duration,
        queues: Option<&[String]>,
    ) -> Result<Option<Reservation>, FrameworkError> {
        if let Some(queues) = queues.filter(|queues| !queues.is_empty()) {
            let unsupported: Vec<&str> = self
                .connections
                .iter()
                .filter(|connection| {
                    connection.driver.queue_filter_capability()
                        == QueueFilterCapability::Unsupported
                })
                .map(|connection| connection.label.as_str())
                .collect();
            if !unsupported.is_empty() {
                return Err(FrameworkError::internal(format!(
                    "failover queue cannot filter --queue={} because these connections do not support queue filtering: {}",
                    queues.join(","),
                    unsupported.join(", ")
                )));
            }
        }

        self.prune_expired_reservations();
        let start = self.next_read.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        let mut failures = Vec::new();

        for offset in 0..self.connections.len() {
            let index = (start + offset) % self.connections.len();
            let connection = Arc::clone(&self.connections[index]);
            let _gate = connection.gate.read().await;
            let pop_started = Instant::now();
            let fallback_deadline = pop_started
                .checked_add(visibility_timeout)
                .unwrap_or(pop_started);
            let popped = match queues {
                Some(queues) => connection.driver.pop_from(visibility_timeout, queues).await,
                None => connection.driver.pop(visibility_timeout).await,
            };
            match popped {
                Ok(Some(reservation)) => {
                    let lease_deadline = connection
                        .driver
                        .reservation_deadline(&reservation.token, fallback_deadline);
                    return Ok(Some(self.remember_reservation(
                        Arc::clone(&connection),
                        reservation,
                        lease_deadline,
                    )));
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        connection = %connection.label,
                        error = %error,
                        "queue connection pop failed"
                    );
                    failures.push((index, format!("connection `{}`: {error}", connection.label)));
                    if queues.is_some_and(|queues| !queues.is_empty())
                        && connection.driver.queue_filter_capability()
                            == QueueFilterCapability::Unknown
                    {
                        failures.sort_unstable_by_key(|(connection_index, _)| *connection_index);
                        return Err(FrameworkError::internal(format!(
                            "failover queue could not confirm queue filtering: {}",
                            failures
                                .into_iter()
                                .map(|(_, failure)| failure)
                                .collect::<Vec<_>>()
                                .join("; ")
                        )));
                    }
                }
            }
        }

        if failures.is_empty() {
            Ok(None)
        } else {
            failures.sort_unstable_by_key(|(connection_index, _)| *connection_index);
            Err(FrameworkError::internal(format!(
                "failover queue pop failed: {}",
                failures
                    .into_iter()
                    .map(|(_, failure)| failure)
                    .collect::<Vec<_>>()
                    .join("; ")
            )))
        }
    }

    fn add_count(total: u64, additional: u64, operation: &str) -> Result<u64, FrameworkError> {
        total.checked_add(additional).ok_or_else(|| {
            FrameworkError::internal(format!(
                "failover queue `{operation}` count exceeds the u64 range"
            ))
        })
    }

    /// Mark `idx` as failing, reporting whether that was a *transition* into
    /// failure rather than a connection already known to be down.
    ///
    /// The check and the set are one locked operation on purpose: a snapshot
    /// read up front would let two concurrent pushes both see a healthy
    /// connection and both announce the same outage.
    ///
    /// A poisoned mutex is recovered rather than propagated: this set decides
    /// whether an event fires, and losing the queue because event bookkeeping
    /// panicked somewhere would be a strictly worse trade.
    fn mark_failing(&self, idx: usize) -> bool {
        self.failing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(idx)
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
        let mut failed = HashSet::new();
        let mut last_err = None;

        for (idx, connection) in self.connections.iter().enumerate() {
            // Cloned per attempt because a refused push consumes the
            // envelope it was handed; the cost is a JSON payload clone
            // against a backend write, which is not where the time goes.
            match connection.driver.push(env.clone()).await {
                Ok(()) => {
                    self.record_failing(failed);
                    return Ok(());
                }
                Err(e) => {
                    let transitioned = self.mark_failing(idx);
                    // `remaining_connections` rather than "falling over to the
                    // next connection", because on the last entry there is no
                    // next one and the push is about to fail outright.
                    let remaining = self.connections.len() - idx - 1;
                    if transitioned {
                        tracing::warn!(
                            connection = %connection.label,
                            job = %env.job_name,
                            error = %e,
                            remaining_connections = remaining,
                            "queue connection refused a push"
                        );
                        // Best-effort, like every other queue event site: a
                        // listener that fails must not fail the push that is
                        // still trying to find a home for this job.
                        let _ = crate::events::EventFacade::dispatch(
                            crate::queue::events::QueueFailedOver {
                                connection: connection.label.clone(),
                                job_name: env.job_name.clone(),
                                exception: e.to_string(),
                            },
                        )
                        .await;
                    } else {
                        // Already-known outage: DEBUG, so the log is as
                        // edge-triggered as the event and a long outage does
                        // not bury everything else at WARN.
                        tracing::debug!(
                            connection = %connection.label,
                            job = %env.job_name,
                            error = %e,
                            remaining_connections = remaining,
                            "queue connection still refusing pushes"
                        );
                    }
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

    // ---- read and lifecycle path: every connection, origin-routed --------

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_rotating(visibility_timeout, None).await
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_rotating(visibility_timeout, Some(queues)).await
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        let mut saw_unknown = false;
        for connection in &self.connections {
            match connection.driver.queue_filter_capability() {
                QueueFilterCapability::Supported => {}
                QueueFilterCapability::Unsupported => {
                    return QueueFilterCapability::Unsupported;
                }
                QueueFilterCapability::Unknown => saw_unknown = true,
            }
        }
        if saw_unknown {
            QueueFilterCapability::Unknown
        } else {
            QueueFilterCapability::Supported
        }
    }

    fn reservation_deadline(
        &self,
        token: &ReservationToken,
        _fallback_deadline: Instant,
    ) -> Instant {
        self.reservation_origin(token)
            .map(|origin| origin.lease_deadline)
            .unwrap_or_else(Instant::now)
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let Some((origin, gate)) = self.guarded_reservation_origin(token).await else {
            return Ok(());
        };
        let result = origin.connection.driver.ack(&origin.inner_token).await;
        if result.is_ok() {
            self.forget_reservation(token);
        }
        drop(gate);
        result
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        let Some((origin, gate)) = self.guarded_reservation_origin(token).await else {
            return Ok(());
        };
        let result = origin
            .connection
            .driver
            .nack(&origin.inner_token, requeue_delay)
            .await;
        if result.is_ok() {
            self.forget_reservation(token);
        }
        drop(gate);
        result
    }

    async fn release(
        &self,
        token: &ReservationToken,
        env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        let Some((origin, gate)) = self.guarded_reservation_origin(token).await else {
            return Ok(());
        };
        let result = origin
            .connection
            .driver
            .release(&origin.inner_token, env, delay)
            .await;
        if result.is_ok() {
            self.forget_reservation(token);
        }
        drop(gate);
        result
    }

    async fn settle(
        &self,
        token: &ReservationToken,
        follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        let Some((origin, gate)) = self.guarded_reservation_origin(token).await else {
            return Ok(Settled::Stale);
        };
        let result = match origin
            .connection
            .driver
            .settle(&origin.inner_token, follow_ups)
            .await
        {
            Ok(Settled::Unsupported) => Ok(Settled::Unsupported),
            Ok(outcome) => {
                self.forget_reservation(token);
                Ok(outcome)
            }
            Err(error) => Err(error),
        };
        drop(gate);
        result
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        let mut total = 0;
        for connection in &self.connections {
            total = Self::add_count(total, connection.driver.size().await?, "size")?;
        }
        Ok(total)
    }

    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        let mut total = 0;
        for connection in &self.connections {
            total = Self::add_count(
                total,
                connection.driver.pending_size().await?,
                "pending_size",
            )?;
        }
        Ok(total)
    }

    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        let mut total = 0;
        for connection in &self.connections {
            total = Self::add_count(
                total,
                connection.driver.delayed_size().await?,
                "delayed_size",
            )?;
        }
        Ok(total)
    }

    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        let mut total = 0;
        for connection in &self.connections {
            total = Self::add_count(
                total,
                connection.driver.reserved_size().await?,
                "reserved_size",
            )?;
        }
        Ok(total)
    }

    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let mut jobs = Vec::new();
        for connection in &self.connections {
            jobs.extend(connection.driver.pending_jobs(queue).await?);
        }
        Ok(jobs)
    }

    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        let mut jobs = Vec::new();
        for connection in &self.connections {
            jobs.extend(connection.driver.delayed_jobs(queue).await?);
        }
        Ok(jobs)
    }

    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        let mut jobs = Vec::new();
        for connection in &self.connections {
            jobs.extend(connection.driver.reserved_jobs(queue).await?);
        }
        Ok(jobs)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        let mut total: u64 = 0;
        let mut failures = Vec::new();
        for connection in &self.connections {
            let _gate = connection.gate.write().await;
            match connection.driver.clear().await {
                Ok(cleared) => {
                    self.forget_connection_reservations(connection);
                    if let Some(next_total) = total.checked_add(cleared) {
                        total = next_total;
                    } else {
                        failures.push(format!(
                            "connection `{}`: clear count overflowed the u64 range",
                            connection.label
                        ));
                    }
                }
                Err(error) => {
                    failures.push(format!("connection `{}`: {error}", connection.label));
                }
            }
        }
        if failures.is_empty() {
            Ok(total)
        } else {
            Err(FrameworkError::internal(format!(
                "failover queue clear failed: {}",
                failures.join("; ")
            )))
        }
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
        let first = Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>;
        let driver = FailoverQueueDriver::new(vec![
            ("one".to_string(), Arc::clone(&first)),
            (
                "two".to_string(),
                Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>,
            ),
        ])
        .expect("two connections");
        assert!(
            Arc::ptr_eq(&first, &driver.connections[0].driver),
            "the first configured connection must remain the write primary"
        );
        assert_eq!(driver.name(), "failover");
    }

    #[tokio::test]
    async fn duplicate_driver_slots_share_one_operation_gate() {
        let shared = Arc::new(MemoryQueueDriver::new()) as Arc<dyn QueueDriver>;
        let driver = FailoverQueueDriver::new(vec![
            ("one".to_string(), Arc::clone(&shared)),
            ("two".to_string(), shared),
        ])
        .expect("duplicate connection slots remain supported");

        assert!(
            Arc::ptr_eq(&driver.connections[0].gate, &driver.connections[1].gate),
            "duplicate slots for one driver must share the pop/clear gate"
        );
    }
}
