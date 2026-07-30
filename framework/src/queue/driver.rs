//! Queue driver trait — the contract every backend implements.

use crate::error::FrameworkError;
use crate::queue::envelope::Envelope;
use async_trait::async_trait;
use chrono::Utc;
use std::time::Duration;
use uuid::Uuid;

/// Opaque token identifying one reservation of a popped envelope.
/// Workers MUST present this token to `ack` or `nack` the message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReservationToken(pub Uuid);

/// One popped message + its reservation token.
#[derive(Debug, Clone)]
pub struct Reservation {
    /// The popped envelope; held until `ack` or `nack` settles the
    /// reservation.
    pub envelope: Envelope,
    /// Driver-issued token the worker presents to settle the message.
    pub token: ReservationToken,
}

/// Backend contract every queue driver (sync, memory, database, redis,
/// SQS, beanstalk, …) implements. The worker speaks to drivers
/// exclusively through this trait.
#[async_trait]
pub trait QueueDriver: Send + Sync {
    /// Enqueue a fully-formed envelope. Drivers MUST NOT mutate it.
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError>;

    /// Pop the next available envelope, reserving it for `visibility_timeout`.
    /// Returns `None` if no message is available within a short driver-local
    /// poll budget. Drivers MAY block up to ~100ms.
    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError>;

    /// Pop the next envelope belonging to one of `queues`.
    ///
    /// An empty slice means "any queue" and MUST behave exactly like
    /// [`QueueDriver::pop`]. This is what a worker started without
    /// `--queue` uses, so the default path is unchanged.
    ///
    /// # Why this errors instead of falling back
    ///
    /// The default implementation rejects a non-empty filter rather than
    /// quietly draining every queue. A worker asked to drain only `billing`
    /// that silently drains everything is indistinguishable from a working
    /// setup until the wrong pool consumes the wrong jobs — a failure that
    /// surfaces in production, not in a smoke test. Drivers that cannot
    /// filter should keep this default so the misconfiguration is loud at
    /// startup.
    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        if queues.is_empty() {
            return self.pop(visibility_timeout).await;
        }
        Err(FrameworkError::internal(format!(
            "queue driver `{}` cannot filter by queue, but the worker was \
             started with --queue={}. Either drop the filter or use a driver \
             that supports routing (memory, database).",
            self.name(),
            queues.join(",")
        )))
    }

    /// Acknowledge successful completion of a reserved message. Drivers MUST
    /// be tolerant of unknown / already-acked tokens (idempotent).
    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError>;

    /// Return a reserved message to the queue with `requeue_delay`.
    ///
    /// **Implementors MUST increment the stored envelope's `attempts`
    /// before re-enqueuing**, so the worker's `attempts >= max_tries`
    /// guard advances correctly across retry cycles. Drivers that store
    /// the envelope server-side (Redis, SQL, etc.) bump on the server;
    /// in-memory drivers bump in their `Inner` map. Failing to bump
    /// causes infinite retry loops.
    ///
    /// Drivers MUST be tolerant of unknown / already-acked tokens (idempotent).
    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError>;

    /// Return a reserved message to the queue after `delay` **without**
    /// consuming an attempt — the retry a job asked for itself via
    /// `Queue::release`, a busy `WithoutOverlapping` lock, or a rate limiter
    /// that wants the work later rather than fewer times.
    ///
    /// The next delivery MUST carry the same `attempts` value this delivery
    /// carried. Drivers that requeue their own stored copy get this for free —
    /// the worker bumps `attempts` on its local envelope only, so the stored
    /// copy still holds the pre-run count. Drivers that re-publish the caller's
    /// `env` must decrement it, which is what the default below does.
    ///
    /// Drivers MUST be tolerant of unknown / already-acked tokens (idempotent).
    ///
    /// # Why this is a driver primitive and not push-then-ack (DATA-02)
    ///
    /// The worker used to release by pushing a copy of the envelope and then
    /// acking the reservation. On any driver that treats the envelope id as a
    /// primary key that is not a release at all: the copy collides with the
    /// still-reserved original, and
    /// [`DatabaseQueueDriver`](crate::queue::database::DatabaseQueueDriver)
    /// returned `UNIQUE constraint failed: jobs.id`. The worker then declined
    /// to ack — correctly, on the evidence it had — so the requested delay was
    /// silently dropped, no `JobReleased` event fired, and the job simply sat
    /// reserved until visibility expiry redelivered it. Every release on a
    /// database-backed queue behaved that way.
    ///
    /// Expressing the release as one driver operation lets each backend do it
    /// in place and atomically, so there is no window in which the message
    /// exists twice or not at all.
    async fn release(
        &self,
        token: &ReservationToken,
        env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        let mut requeued = env.clone();
        requeued.attempts = requeued.attempts.saturating_sub(1);
        requeued.available_at = Utc::now()
            + chrono::Duration::from_std(delay).unwrap_or_else(|_| chrono::Duration::zero());
        self.push(requeued).await?;
        self.ack(token).await
    }

    /// Total count of envelopes currently held by this driver
    /// (pending + delayed + reserved). Mirrors Laravel's
    /// `Queue::size($queue)`.
    ///
    /// Default implementation returns `Err` describing the unsupported
    /// operation — drivers that can answer the count cheaply override.
    async fn size(&self) -> Result<u64, FrameworkError> {
        Err(FrameworkError::internal(format!(
            "queue driver '{}' does not implement size()",
            self.name()
        )))
    }

    /// Count of envelopes whose `available_at <= now` and which are not
    /// currently reserved. Mirrors Laravel's `pendingSize($queue)`.
    /// Defaults to [`size`](Self::size) minus the reserved/delayed counts.
    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        let total = self.size().await?;
        let reserved = self.reserved_size().await.unwrap_or(0);
        let delayed = self.delayed_size().await.unwrap_or(0);
        Ok(total.saturating_sub(reserved).saturating_sub(delayed))
    }

    /// Count of envelopes whose `available_at > now`. Mirrors
    /// `delayedSize($queue)`.
    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    /// Count of currently-reserved envelopes (popped, not yet acked).
    /// Mirrors `reservedSize($queue)`.
    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    /// Drop every envelope, returning the number removed. Mirrors
    /// `Queue::clear($queue)` and the `ClearableQueue` contract.
    async fn clear(&self) -> Result<u64, FrameworkError> {
        Err(FrameworkError::internal(format!(
            "queue driver '{}' does not implement clear()",
            self.name()
        )))
    }

    /// Push every envelope in one shot. Mirrors `Queue::bulk($jobs, ...)`.
    /// Default implementation pushes serially; backends with native bulk
    /// push (sea-streamer pipeline, DB multi-row insert) may override.
    async fn bulk_push(&self, envs: Vec<Envelope>) -> Result<(), FrameworkError> {
        for env in envs {
            self.push(env).await?;
        }
        Ok(())
    }

    /// Driver name for logs/admin. Default uses type name.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}
