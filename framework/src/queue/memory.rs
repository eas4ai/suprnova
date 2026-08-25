//! In-memory queue driver.
//!
//! Canonical test surface. Backed by:
//! - a `VecDeque<Envelope>` for the visible queue,
//! - a `HashMap<ReservationToken, Envelope>` for reservations,
//! - a `tokio_util::time::DelayQueue<ReservationToken>` for visibility-timeout expiry,
//! - a `DelayedStore` (`tokio_util::time::DelayQueue<Uuid>` plus an
//!   id-keyed `HashMap<Uuid, Envelope>`) for delayed jobs - split so the
//!   delayed set is listable, which a bare `DelayQueue<Envelope>` is not.
//!
//! # Design note - paused-clock compatibility
//!
//! Both DelayQueues run on Tokio's virtual clock. Under
//! `#[tokio::test(start_paused = true)]`, `tokio::time::advance(N)` correctly
//! fires their expirations, so paused-clock tests for delayed jobs work without
//! any wall-clock comparison.
//!
//! `pop` drains both DelayQueues synchronously (via a noop-waker context) before
//! checking the visible queue. This means that even when the background reaper's
//! `sleep(50ms)` never fires, reclaim and delayed-job promotion both happen on
//! the next `pop` call after the caller has advanced the virtual clock.
//!
//! The reaper is retained for production use where `pop` is infrequent
//! and background reclaim is needed.

use crate::error::FrameworkError;
use crate::lock;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken};
use crate::queue::envelope::{Envelope, queue_filter, queue_matches};
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::time::DelayQueue;
use uuid::Uuid;

#[derive(Default)]
struct Inner {
    visible: VecDeque<Envelope>,
    reserved: HashMap<ReservationToken, Envelope>,
}

/// Delayed-job storage: `queue` wakes envelope ids on Tokio's virtual-clock
/// timer wheel, `by_id` owns the actual envelopes.
///
/// The two are split because `DelayQueue<T>` has no iteration API - only
/// expiry polling - so a bare `DelayQueue<Envelope>` (the pre-inspection-API
/// shape) cannot be listed. Keying the timer wheel on `Uuid` and moving
/// ownership of the envelope into `by_id` is what makes `delayed_jobs()`
/// possible: the map is directly iterable, and a wake just looks up and
/// removes the id it names.
#[derive(Default)]
struct DelayedStore {
    queue: DelayQueue<Uuid>,
    by_id: HashMap<Uuid, Envelope>,
}

impl DelayedStore {
    /// Park `env` for `delay`, recorded under its own id in both halves.
    fn insert(&mut self, env: Envelope, delay: Duration) {
        self.queue.insert(env.id, delay);
        self.by_id.insert(env.id, env);
    }

    /// Number of envelopes currently parked. `by_id` is authoritative -
    /// every `insert` and every successful drain keeps the two in lockstep.
    fn len(&self) -> usize {
        self.by_id.len()
    }

    fn clear(&mut self) {
        self.queue.clear();
        self.by_id.clear();
    }
}

/// In-process [`QueueDriver`] backed by a FIFO `VecDeque` plus
/// `DelayQueue`s for visibility timeouts and delayed dispatches.
/// Lost on process restart.
pub struct MemoryQueueDriver {
    inner: Arc<Mutex<Inner>>,
    /// Async mutex guards the visibility DelayQueue so both `pop` and the reaper
    /// can poll it synchronously after acquiring the lock.
    visibility: Arc<AsyncMutex<DelayQueue<ReservationToken>>>,
    /// Async mutex guards [`DelayedStore`] - runs on Tokio's virtual clock so
    /// `tokio::time::advance` correctly fires expirations in paused-clock tests.
    delayed: Arc<AsyncMutex<DelayedStore>>,
    reaper: tokio::task::JoinHandle<()>,
}

impl Drop for MemoryQueueDriver {
    fn drop(&mut self) {
        self.reaper.abort();
    }
}

/// Drain all currently-expired visibility reservations from `dq` back into
/// the visible queue (push_front - reservation reclaim is priority).
/// The noop waker context must be created and dropped within this call -
/// callers must ensure it is not held across an await.
fn drain_expired(
    inner: &Mutex<Inner>,
    dq: &mut DelayQueue<ReservationToken>,
) -> Result<(), FrameworkError> {
    let waker = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    let mut expired_tokens = Vec::new();
    while let Poll::Ready(Some(item)) = dq.poll_expired(&mut cx) {
        expired_tokens.push(item.into_inner());
    }
    // cx / waker are dropped here - no await has occurred.
    if !expired_tokens.is_empty() {
        let mut g = lock::lock(inner, "memory queue state")?;
        for token in expired_tokens {
            if let Some(mut env) = g.reserved.remove(&token) {
                // A reservation reaching here lapsed without being settled:
                // the worker holding it never acked, nacked or released. It
                // died mid-handler. That is a consumed attempt, and it has
                // to be counted here because nothing else will - a job that
                // *fails* is nacked and counted by `requeue`, but a job that
                // *kills its worker* settles nothing. Leaving the count
                // alone makes such a job immortal: it kills each worker
                // that claims it, is redelivered unchanged, and kills the
                // next one.
                //
                // The database driver counts the same event in its reclaim
                // path; the semantics have to match, because swapping the
                // driver must not change whether a poison job can be
                // dead-lettered.
                env.attempts += 1;
                g.visible.push_front(env);
            }
        }
    }
    Ok(())
}

/// Drain all currently-expired delayed envelopes from `store` into the
/// visible queue (push_back - delayed jobs join the back of the FIFO line).
/// The noop waker context must be created and dropped within this call -
/// callers must ensure it is not held across an await.
fn drain_delayed(inner: &Mutex<Inner>, store: &mut DelayedStore) -> Result<(), FrameworkError> {
    let waker = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    let mut ready_ids = Vec::new();
    while let Poll::Ready(Some(item)) = store.queue.poll_expired(&mut cx) {
        ready_ids.push(item.into_inner());
    }
    // cx / waker are dropped here - no await has occurred.
    if !ready_ids.is_empty() {
        let mut g = lock::lock(inner, "memory queue state")?;
        for id in ready_ids {
            // A wake whose id is no longer in `by_id` was already promoted
            // or cleared (e.g. by a concurrent drain, or by `clear()`) - the
            // timer firing for it now is a stale echo, not new work.
            if let Some(env) = store.by_id.remove(&id) {
                g.visible.push_back(env);
            }
        }
    }
    Ok(())
}

impl MemoryQueueDriver {
    /// Construct a fresh in-process queue driver. Spawns a Tokio reaper
    /// task that reclaims expired visibility reservations; the task is
    /// aborted when the driver is dropped.
    pub fn new() -> Self {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let visibility = Arc::new(AsyncMutex::new(DelayQueue::new()));
        let delayed: Arc<AsyncMutex<DelayedStore>> =
            Arc::new(AsyncMutex::new(DelayedStore::default()));

        let inner2 = inner.clone();
        let visibility2 = visibility.clone();
        let delayed2 = delayed.clone();

        let reaper = tokio::spawn(async move {
            loop {
                // Promote expired delayed jobs into the visible queue.
                // Log poison/internal errors but DO NOT abort the reaper -
                // a single panicking producer must not strand every
                // delayed job in the queue. The reaper backs off via the
                // normal 50ms sleep below before the next attempt.
                {
                    let mut store = delayed2.lock().await;
                    if let Err(e) = drain_delayed(&inner2, &mut store) {
                        tracing::error!(
                            error = %e,
                            "memory queue reaper: drain_delayed failed; continuing"
                        );
                    }
                }

                // Reclaim expired visibility reservations.
                {
                    let mut dq = visibility2.lock().await;
                    if let Err(e) = drain_expired(&inner2, &mut dq) {
                        tracing::error!(
                            error = %e,
                            "memory queue reaper: drain_expired failed; continuing"
                        );
                    }
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });

        Self {
            inner,
            visibility,
            delayed,
            reaper,
        }
    }
}

impl Default for MemoryQueueDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl QueueDriver for MemoryQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        let now = Utc::now();
        if env.available_at <= now {
            let mut g = lock::lock(&self.inner, "memory queue state")?;
            g.visible.push_back(env);
        } else {
            // Compute delay on the Tokio virtual clock so paused-clock tests work.
            let delay = (env.available_at - now).to_std().unwrap_or(Duration::ZERO);
            let mut store = self.delayed.lock().await;
            store.insert(env, delay);
        }
        Ok(())
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, queues).await
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, &[]).await
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let mut g = lock::lock(&self.inner, "memory queue state")?;
        g.reserved.remove(token);
        Ok(())
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, true).await
    }

    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        // The reserved copy still holds the pre-run attempt count - the worker
        // bumps only its own local envelope - so requeuing it without a bump
        // is exactly "try again without burning an attempt".
        self.requeue(token, delay, false).await
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        let visible = {
            let g = lock::lock(&self.inner, "memory queue state")?;
            (g.visible.len() + g.reserved.len()) as u64
        };
        let delayed = self.delayed.lock().await.len() as u64;
        Ok(visible + delayed)
    }

    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        let g = lock::lock(&self.inner, "memory queue state")?;
        Ok(g.visible.len() as u64)
    }

    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        Ok(self.delayed.lock().await.len() as u64)
    }

    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        let g = lock::lock(&self.inner, "memory queue state")?;
        Ok(g.reserved.len() as u64)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        let dropped_visible_reserved = {
            let mut g = lock::lock(&self.inner, "memory queue state")?;
            let n = (g.visible.len() + g.reserved.len()) as u64;
            g.visible.clear();
            g.reserved.clear();
            n
        };
        let delayed_dropped = {
            let mut store = self.delayed.lock().await;
            let n = store.len() as u64;
            store.clear();
            n
        };
        // Visibility DelayQueue is reservation accounting only - clearing
        // the visible/reserved maps makes its expirations no-ops, but
        // emptying it too prevents stale reservation tokens from firing
        // future reclaim events.
        self.visibility.lock().await.clear();
        Ok(dropped_visible_reserved + delayed_dropped)
    }

    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.drain_all().await?;
        let filter = queue_filter(queue);
        let g = lock::lock(&self.inner, "memory queue state")?;
        Ok(g.visible
            .iter()
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(InspectedJob::from_envelope)
            .collect())
    }

    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.drain_all().await?;
        let filter = queue_filter(queue);
        let store = self.delayed.lock().await;
        Ok(store
            .by_id
            .values()
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(InspectedJob::from_envelope)
            .collect())
    }

    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.drain_all().await?;
        let filter = queue_filter(queue);
        let g = lock::lock(&self.inner, "memory queue state")?;
        Ok(g.reserved
            .values()
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(InspectedJob::from_envelope)
            .collect())
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}

impl MemoryQueueDriver {
    /// Shared body of [`QueueDriver::nack`] and [`QueueDriver::release`],
    /// which differ only in whether the requeue consumes an attempt.
    ///
    /// Taking the envelope out of `reserved` and putting it back is one
    /// operation from any caller's point of view: the message is never
    /// simultaneously reserved and visible, and never neither.
    async fn requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        consume_attempt: bool,
    ) -> Result<(), FrameworkError> {
        let env = {
            let mut g = lock::lock(&self.inner, "memory queue state")?;
            g.reserved.remove(token)
        };
        if let Some(mut env) = env {
            if consume_attempt {
                env.attempts += 1;
            }
            if delay.is_zero() {
                let mut g = lock::lock(&self.inner, "memory queue state")?;
                g.visible.push_front(env);
            } else {
                env.available_at = Utc::now()
                    + chrono::Duration::from_std(delay).map_err(|e| {
                        FrameworkError::internal(format!("requeue delay overflow: {e}"))
                    })?;
                // Insert into the Tokio-virtual-clock DelayedStore.
                let mut store = self.delayed.lock().await;
                store.insert(env, delay);
            }
        }
        Ok(())
    }

    /// Drain both DelayQueues - delayed-job promotion, then reservation
    /// reclaim - so `inner` reflects exactly what the next `pop` would see.
    ///
    /// Shared by [`pop_filtered`](Self::pop_filtered) and the
    /// `pending_jobs`/`delayed_jobs`/`reserved_jobs` listings: without this,
    /// a delayed job whose `available_at` had already passed but whose
    /// 50ms-interval reaper tick hadn't yet run would show up in
    /// `delayed_jobs()` even though a `pop` right after would have returned
    /// it as pending.
    async fn drain_all(&self) -> Result<(), FrameworkError> {
        {
            let mut store = self.delayed.lock().await;
            drain_delayed(&self.inner, &mut store)?;
            // store lock released here.
        }
        {
            let mut dq = self.visibility.lock().await;
            drain_expired(&self.inner, &mut dq)?;
            // dq lock released here.
        }
        Ok(())
    }

    /// Shared body of [`QueueDriver::pop`] and [`QueueDriver::pop_from`].
    ///
    /// An empty `queues` scans nothing and pops the head, which keeps the
    /// unfiltered path exactly as it was before routing existed.
    async fn pop_filtered(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.drain_all().await?;

        let env_opt = {
            let mut g = lock::lock(&self.inner, "memory queue state")?;
            if queues.is_empty() {
                g.visible.pop_front()
            } else {
                // Scan for the first envelope this worker is allowed to take.
                // Order is preserved for the queues being drained; envelopes
                // for other queues stay put rather than being consumed and
                // re-queued, so a filtered worker never perturbs FIFO order
                // for the pool that owns them.
                let idx = g
                    .visible
                    .iter()
                    .position(|e| queue_matches(e.queue.as_deref(), queues));
                match idx {
                    Some(i) => g.visible.remove(i),
                    None => None,
                }
            }
        };

        if let Some(env) = env_opt {
            let token = ReservationToken(Uuid::new_v4());
            {
                let mut g = lock::lock(&self.inner, "memory queue state")?;
                g.reserved.insert(token.clone(), env.clone());
            }
            self.visibility
                .lock()
                .await
                .insert(token.clone(), visibility_timeout);
            Ok(Some(Reservation {
                envelope: env,
                token,
            }))
        } else {
            Ok(None)
        }
    }
}
