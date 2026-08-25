//! Queue driver trait - the contract every backend implements.

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

/// What [`QueueDriver::settle`] did.
///
/// Settlement has three genuinely different endings and the worker must react
/// differently to each, so they are named rather than squeezed into a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The follow-up envelopes were enqueued and the reservation was dropped
    /// as one atomic step. There is no window in which one happened without
    /// the other.
    Atomically,
    /// The reservation was no longer this worker's - visibility expired and
    /// another consumer reclaimed the message, or a previous settlement
    /// already committed. **Nothing was enqueued and nothing was dropped.**
    ///
    /// This is the outcome that makes the protocol worth having: a worker that
    /// comes back from a long GC pause or a stalled write cannot enqueue a
    /// chain successor for a job somebody else now owns.
    Stale,
    /// The driver has no transactional settlement. The caller must fall back
    /// to pushing the follow-ups and then acking, accepting the duplicate
    /// window that implies.
    Unsupported,
}

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
    /// setup until the wrong pool consumes the wrong jobs - a failure that
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
    /// consuming an attempt - the retry a job asked for itself via
    /// `Queue::release`, a busy `WithoutOverlapping` lock, or a rate limiter
    /// that wants the work later rather than fewer times.
    ///
    /// The next delivery MUST carry the same `attempts` value this delivery
    /// carried. Drivers that requeue their own stored copy get this for free -
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
    /// to ack - correctly, on the evidence it had - so the requested delay was
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

    /// Enqueue `follow_ups` and drop the reservation held by `token` as a
    /// single atomic step - the outbox half of terminal settlement (DATA-02).
    ///
    /// Returns [`Settled::Unsupported`] by default, which tells the worker to
    /// fall back to push-then-ack. Implement it on any backend where the
    /// follow-up write and the acknowledgement share a transaction domain.
    ///
    /// # The window this closes
    ///
    /// A worker that finishes a chained job has to do two things: enqueue the
    /// next link, and release the job it just finished. Done as two operations
    /// there is no safe order:
    ///
    /// - Ack first, and a crash before the push loses the rest of the chain
    ///   permanently - nothing is left in the queue to retry from.
    /// - Push first (what the worker does when this returns `Unsupported`),
    ///   and a crash before the ack redelivers the finished job, which runs
    ///   its handler a second time and pushes the successor again.
    ///
    /// The second is the safer trade and the framework takes it, because
    /// at-least-once delivery is the documented contract. But "safer" is not
    /// "correct": the duplicate is real, and on a chain it means the successor
    /// can run twice.
    ///
    /// One transaction removes the choice. Either both effects commit or
    /// neither does, and a redelivery after a rollback replays a job whose
    /// successor was never enqueued.
    ///
    /// # Contract for implementors
    ///
    /// - Enqueue every envelope in `follow_ups`, then drop the reservation,
    ///   and commit them together. Partial success MUST NOT be observable.
    /// - If the reservation is not (or is no longer) held by `token`, commit
    ///   nothing and return [`Settled::Stale`]. Do **not** treat this as the
    ///   idempotent no-op [`ack`](Self::ack) is allowed to be: the caller is
    ///   about to enqueue work on behalf of a message it does not own.
    /// - An empty `follow_ups` is a plain acknowledgement and must still
    ///   report `Stale` rather than success when the reservation is gone.
    /// - Return `Err` only for genuine backend failures. The caller leaves the
    ///   reservation intact and lets visibility expiry redeliver.
    async fn settle(
        &self,
        token: &ReservationToken,
        follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        let _ = (token, follow_ups);
        Ok(Settled::Unsupported)
    }

    /// Total count of envelopes currently held by this driver
    /// (pending + delayed + reserved). Mirrors Laravel's
    /// `Queue::size($queue)`.
    ///
    /// Default implementation returns `Err` describing the unsupported
    /// operation - drivers that can answer the count cheaply override.
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
