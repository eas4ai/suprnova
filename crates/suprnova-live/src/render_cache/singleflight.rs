//! Fenced rebuild coordination: one accepted publication per key and
//! coherence fence, bounded waiters, expiry, and release.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;

use super::key::RenderKey;
use super::store::PublicationFence;
use super::{RenderCacheError, RenderCacheErrorKind};

/// A held rebuild lease; consumed by [`RebuildCoordinator::publish_token`] or
/// [`RebuildCoordinator::release`].
#[derive(Debug)]
pub struct RebuildLease {
    key: RenderKey,
    epoch: u64,
    lease_id: u64,
    expires_at_ms: u64,
}

impl RebuildLease {
    /// The key this lease rebuilds.
    #[must_use]
    pub fn key(&self) -> &RenderKey {
        &self.key
    }
}

/// Shared completion state for one rebuild slot. Executor-neutral: the
/// engine never depends on a runtime, so this is a hand-written one-shot
/// notification over the standard library rather than a channel.
#[derive(Debug, Default)]
struct Completion {
    done: bool,
    wakers: Vec<Waker>,
}

#[derive(Clone, Debug, Default)]
struct CompletionHandle(Arc<Mutex<Completion>>);

impl CompletionHandle {
    /// Marks the slot finished and wakes every registered waiter exactly
    /// once. Wakers are drained and called only after the lock is released,
    /// so a waker that re-enters this coordinator on the same thread cannot
    /// deadlock against the lock this method holds.
    fn complete(&self) {
        let wakers = {
            let mut state = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.done = true;
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }
}

/// A waiter's handle: resolves when the leader publishes or releases.
#[derive(Clone, Debug)]
pub struct RebuildWait {
    completion: CompletionHandle,
}

impl RebuildWait {
    /// Waits for the leader; returns when it published or released.
    pub async fn wait(self) {
        self.await;
    }
}

impl Future for RebuildWait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self
            .completion
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.done {
            return Poll::Ready(());
        }
        // Registering under the same lock that `complete` takes is what
        // makes a wakeup impossible to miss: a completion racing this poll
        // either observes this waker (and will wake it) or has already set
        // `done` (and this poll observes that instead).
        if !state
            .wakers
            .iter()
            .any(|waker| waker.will_wake(context.waker()))
        {
            state.wakers.push(context.waker().clone());
        }
        Poll::Pending
    }
}

/// The admission decision for a rebuild attempt.
#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "boxing Lead would force release() to take Box<RebuildLease>, widening the \
              trait's owned-lease contract; the leader path already pays one allocation \
              per rebuild, so the extra stack bytes here are the cheaper trade"
)]
pub enum RebuildAdmission {
    /// This request rebuilds.
    Lead(RebuildLease),
    /// Another request rebuilds; wait for it within bounds.
    Wait(RebuildWait),
    /// Waiters are exhausted; render without publishing.
    Bypass,
}

/// Provider contract for rebuild coordination.
#[async_trait]
pub trait RebuildCoordinator: Send + Sync {
    /// Admits a rebuild for a key at an epoch.
    async fn admit(
        &self,
        key: &RenderKey,
        epoch: u64,
        now_ms: u64,
    ) -> Result<RebuildAdmission, RenderCacheError>;
    /// Mints the publication fence for a current lease; fenced leases fail.
    async fn publish_token(
        &self,
        lease: &RebuildLease,
        now_ms: u64,
    ) -> Result<PublicationFence, RenderCacheError>;
    /// Releases a lease without or after publishing; wakes waiters.
    async fn release(&self, lease: RebuildLease) -> Result<(), RenderCacheError>;
}

/// Bounds of the in-process coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalCoordinatorLimits {
    /// Lease lifetime in milliseconds.
    pub lease_ms: u64,
    /// Most waiters admitted per key.
    pub max_waiters: usize,
}

struct Slot {
    epoch: u64,
    lease_id: u64,
    expires_at_ms: u64,
    waiters: usize,
    completion: CompletionHandle,
}

struct CoordinatorState {
    slots: BTreeMap<RenderKey, Slot>,
    next_lease_id: u64,
    next_token: u64,
}

/// Single-process coordinator: the Tier 0 reference.
pub struct LocalRebuildCoordinator {
    limits: LocalCoordinatorLimits,
    state: Mutex<CoordinatorState>,
}

impl LocalRebuildCoordinator {
    /// Creates an idle coordinator.
    #[must_use]
    pub fn new(limits: LocalCoordinatorLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(CoordinatorState {
                slots: BTreeMap::new(),
                next_lease_id: 0,
                next_token: 0,
            }),
        }
    }

    /// Locks the state, recovering it from poison rather than propagating a
    /// panic across this coordinator's operations.
    fn lock_state(&self) -> MutexGuard<'_, CoordinatorState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl RebuildCoordinator for LocalRebuildCoordinator {
    async fn admit(
        &self,
        key: &RenderKey,
        epoch: u64,
        now_ms: u64,
    ) -> Result<RebuildAdmission, RenderCacheError> {
        let mut state = self.lock_state();
        if let Some(slot) = state.slots.get_mut(key) {
            let superseded = epoch > slot.epoch || now_ms >= slot.expires_at_ms;
            if !superseded {
                if slot.waiters >= self.limits.max_waiters {
                    return Ok(RebuildAdmission::Bypass);
                }
                slot.waiters += 1;
                return Ok(RebuildAdmission::Wait(RebuildWait {
                    completion: slot.completion.clone(),
                }));
            }
            // A newer epoch or an expired lease replaces this slot; wake any
            // waiters that were waiting on the outgoing leader before it is
            // overwritten below.
            slot.completion.complete();
        }
        state.next_lease_id += 1;
        let lease_id = state.next_lease_id;
        let expires_at_ms = now_ms.saturating_add(self.limits.lease_ms);
        state.slots.insert(
            key.clone(),
            Slot {
                epoch,
                lease_id,
                expires_at_ms,
                waiters: 0,
                completion: CompletionHandle::default(),
            },
        );
        Ok(RebuildAdmission::Lead(RebuildLease {
            key: key.clone(),
            epoch,
            lease_id,
            expires_at_ms,
        }))
    }

    async fn publish_token(
        &self,
        lease: &RebuildLease,
        now_ms: u64,
    ) -> Result<PublicationFence, RenderCacheError> {
        let mut state = self.lock_state();
        let current = state.slots.get(&lease.key).filter(|slot| {
            slot.lease_id == lease.lease_id
                && now_ms < slot.expires_at_ms
                && now_ms < lease.expires_at_ms
        });
        if current.is_none() {
            return Err(RenderCacheError::new(
                RenderCacheErrorKind::PublicationFenced,
            ));
        }
        state.next_token += 1;
        Ok(PublicationFence {
            epoch: lease.epoch,
            generation_digest: [0; 32],
            token: state.next_token,
        })
    }

    async fn release(&self, lease: RebuildLease) -> Result<(), RenderCacheError> {
        let mut state = self.lock_state();
        if let Some(slot) = state.slots.get(&lease.key)
            && slot.lease_id == lease.lease_id
        {
            slot.completion.complete();
            state.slots.remove(&lease.key);
        }
        Ok(())
    }
}
