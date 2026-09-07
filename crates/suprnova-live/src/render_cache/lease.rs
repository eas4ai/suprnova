//! Cross-node rebuild leadership: the [`LeaseStore`] port, an in-memory
//! reference implementation of it, and [`FencedLeaseCoordinator`], the kernel
//! that composes a [`LocalRebuildCoordinator`] (in-process waiters, unchanged)
//! with a store (leadership across processes).
//!
//! Two rules hold this module together.
//!
//! **No cross-node waiting.** When the store reports that another node holds
//! the key, this coordinator releases the local lease it just took (so the
//! waiters parked behind it wake and re-admit) and answers
//! [`RebuildAdmission::Bypass`]: the request renders without publishing.
//! Bounded duplicate computation across nodes is permitted by the rebuild
//! contract; two accepted publications are not, and the store's fence is what
//! prevents them. Nothing here polls or sleeps waiting for another node.
//!
//! **Store time decides every distributed question.** A lease expires, is
//! taken over, or mints a token by the backing store's own clock, read inside
//! the store operation. The `now_ms` arguments to the [`RebuildCoordinator`]
//! methods are node time and feed only the local coordinator, so a node whose
//! clock is skewed can neither extend nor shorten the lease it holds in the
//! store.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;

use crate::clock::Clock;
use crate::identity::UnixMillis;

use super::key::RenderKey;
use super::singleflight::{
    LocalCoordinatorLimits, LocalRebuildCoordinator, RebuildAdmission, RebuildCoordinator,
    RebuildLease,
};
use super::store::PublicationFence;
use super::{RenderCacheError, RenderCacheErrorKind};

/// The outcome of one attempt to take a key's lease in a store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseAttempt {
    /// The caller now holds the lease.
    Acquired {
        /// Store-issued identity of this tenure; a later tenure never reuses
        /// it, which is what makes a fenced-out holder detectable.
        lease_id: u64,
        /// Expiry in the store's own milliseconds-since-epoch, for
        /// inspection. The store, not the caller, enforces it.
        expires_at_ms: u64,
    },
    /// An unexpired lease belongs to someone else; the caller leads nothing.
    Held,
}

/// Provider contract for cross-node rebuild leadership. A backend implements
/// only atomicity and time: every operation is decided against the store's own
/// clock, inside the same atomic step as the state it guards.
#[async_trait]
pub trait LeaseStore: Send + Sync {
    /// Acquires the lease for `key` at `epoch` when no unexpired lease holds
    /// it by store time, taking over an expired one. A newer `epoch` never
    /// preempts an unexpired lease: the requester bypasses instead, for at
    /// most the remaining `ttl_ms` of the current tenure.
    async fn try_acquire(
        &self,
        key: &RenderKey,
        epoch: u64,
        ttl_ms: u64,
    ) -> Result<LeaseAttempt, RenderCacheError>;
    /// Mints the next publication token for `key` when `lease_id` still holds
    /// an unexpired lease on it by store time; otherwise `None`. Tokens are
    /// monotonic per key across tenures, so a token minted under an older
    /// lease can never outrank one minted under a newer lease.
    async fn mint_token(
        &self,
        key: &RenderKey,
        lease_id: u64,
    ) -> Result<Option<u64>, RenderCacheError>;
    /// Releases the lease on `key` when `lease_id` still holds it. Releasing a
    /// lease that has already expired or been taken over changes nothing.
    async fn release(&self, key: &RenderKey, lease_id: u64) -> Result<(), RenderCacheError>;
}

/// One key's lease row in [`MemoryLeaseStore`].
struct LeaseSlot {
    /// Authority epoch the current or last holder acquired at; recorded for
    /// inspection, never a condition of acquisition.
    epoch: u64,
    /// The holder's lease id, or `None` when nobody holds the key.
    lease_id: Option<u64>,
    /// Store-time expiry of the current holder's tenure.
    expires_at_ms: u64,
    /// The token the next successful mint on this key returns. It survives
    /// release and takeover: were it to restart, a fenced-out holder's already
    /// published fence could outrank the publication that replaces it.
    next_token: u64,
}

/// In-process reference implementation of [`LeaseStore`] over an injected
/// [`Clock`], which stands in for the backing store's clock.
///
/// It is the conformance reference for the port and the test double for the
/// kernel. Distributed adapters implement the same semantics against a
/// database row or a Redis key; this one keeps a small slot per key that has
/// ever been leased, because the per-key token counter must outlive every
/// tenure.
pub struct MemoryLeaseStore {
    clock: Arc<dyn Clock>,
    slots: Mutex<BTreeMap<RenderKey, LeaseSlot>>,
    next_lease_id: AtomicU64,
}

impl MemoryLeaseStore {
    /// Creates an empty store that reads store time from `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            slots: Mutex::new(BTreeMap::new()),
            next_lease_id: AtomicU64::new(0),
        }
    }

    /// Reads store time, reporting a clock the host cannot read as a provider
    /// failure rather than guessing a timestamp that would decide expiry.
    ///
    /// Callers read it while they hold the slots, so the time an operation
    /// decides by and the state it decides over are one step, as a backend's
    /// own clock inside its own statement would be.
    fn store_now_ms(&self) -> Result<u64, RenderCacheError> {
        self.clock
            .now()
            .map(UnixMillis::get)
            .map_err(|_| RenderCacheError::new(RenderCacheErrorKind::ProviderUnavailable))
    }

    /// Returns the authority epoch recorded for `key`'s current or last
    /// tenure, or `None` when the key has never been leased. A distributed
    /// adapter keeps the same column; nothing about acquisition depends on
    /// it, so reading it observes leadership without deciding it.
    #[must_use]
    pub fn epoch(&self, key: &RenderKey) -> Option<u64> {
        self.lock_slots().get(key).map(|slot| slot.epoch)
    }

    /// Locks the slots, recovering them from poison rather than propagating a
    /// panic across this store's operations.
    fn lock_slots(&self) -> MutexGuard<'_, BTreeMap<RenderKey, LeaseSlot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl LeaseStore for MemoryLeaseStore {
    async fn try_acquire(
        &self,
        key: &RenderKey,
        epoch: u64,
        ttl_ms: u64,
    ) -> Result<LeaseAttempt, RenderCacheError> {
        let mut slots = self.lock_slots();
        let now_ms = self.store_now_ms()?;
        let slot = slots.entry(key.clone()).or_insert(LeaseSlot {
            epoch,
            lease_id: None,
            expires_at_ms: 0,
            next_token: 0,
        });
        if slot.lease_id.is_some() && now_ms < slot.expires_at_ms {
            return Ok(LeaseAttempt::Held);
        }
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::SeqCst) + 1;
        let expires_at_ms = now_ms.saturating_add(ttl_ms);
        slot.epoch = epoch;
        slot.lease_id = Some(lease_id);
        slot.expires_at_ms = expires_at_ms;
        Ok(LeaseAttempt::Acquired {
            lease_id,
            expires_at_ms,
        })
    }

    async fn mint_token(
        &self,
        key: &RenderKey,
        lease_id: u64,
    ) -> Result<Option<u64>, RenderCacheError> {
        let mut slots = self.lock_slots();
        let now_ms = self.store_now_ms()?;
        let Some(slot) = slots.get_mut(key) else {
            return Ok(None);
        };
        if slot.lease_id != Some(lease_id) || now_ms >= slot.expires_at_ms {
            return Ok(None);
        }
        slot.next_token += 1;
        Ok(Some(slot.next_token))
    }

    async fn release(&self, key: &RenderKey, lease_id: u64) -> Result<(), RenderCacheError> {
        let mut slots = self.lock_slots();
        if let Some(slot) = slots.get_mut(key)
            && slot.lease_id == Some(lease_id)
        {
            slot.lease_id = None;
            slot.expires_at_ms = 0;
        }
        Ok(())
    }
}

/// Rebuild coordinator for several processes over one [`LeaseStore`]: the
/// in-process coordinator still parks local waiters behind their leader, and
/// the store decides which process leads.
pub struct FencedLeaseCoordinator<S: LeaseStore> {
    store: Arc<S>,
    local: LocalRebuildCoordinator,
}

impl<S: LeaseStore> FencedLeaseCoordinator<S> {
    /// Creates a coordinator over `store`, admitting in-process requests under
    /// `local`. The same lease lifetime bounds the store's lease.
    #[must_use]
    pub fn new(store: Arc<S>, local: LocalCoordinatorLimits) -> Self {
        Self {
            store,
            local: LocalRebuildCoordinator::new(local),
        }
    }
}

#[async_trait]
impl<S: LeaseStore> RebuildCoordinator for FencedLeaseCoordinator<S> {
    async fn admit(
        &self,
        key: &RenderKey,
        epoch: u64,
        now_ms: u64,
    ) -> Result<RebuildAdmission, RenderCacheError> {
        let admission = self.local.admit(key, epoch, now_ms).await?;
        let RebuildAdmission::Lead(lease) = admission else {
            return Ok(admission);
        };
        let ttl_ms = self.local.limits().lease_ms;
        match self.store.try_acquire(key, epoch, ttl_ms).await {
            Ok(LeaseAttempt::Acquired { lease_id, .. }) => Ok(RebuildAdmission::Lead(Box::new(
                lease.with_distributed_lease_id(lease_id),
            ))),
            // Another node leads. Handing back the local lease is what keeps
            // this node's own waiters honest: they wake, re-admit, and one of
            // them takes the next turn at the store instead of parking behind
            // a leader that will never publish.
            Ok(LeaseAttempt::Held) => {
                self.local.release(*lease).await?;
                Ok(RebuildAdmission::Bypass)
            }
            // The store decided nothing, so this node leads nothing: hand the
            // local lease back before reporting the failure, or the waiters
            // behind it would park on a leader that will never publish.
            //
            // The hand-back propagates with `?`, exactly as the `Held` arm
            // above does, and for the same reason: a local release that
            // failed would leave this node's waiters parked behind a leader
            // that is not coming back, which is a worse answer than the
            // store's own. Either way admission fails closed - the caller
            // renders without caching - so the difference is only which
            // failure the caller is told about, and a coordinator that
            // cannot release its own lease is the more urgent one.
            Err(error) => {
                self.local.release(*lease).await?;
                Err(error)
            }
        }
    }

    async fn publish_token(
        &self,
        lease: &RebuildLease,
        _now_ms: u64,
    ) -> Result<PublicationFence, RenderCacheError> {
        // Store time alone: a node's `now_ms` says nothing about whether the
        // store still holds this lease.
        let Some(lease_id) = lease.distributed_lease_id() else {
            return Err(RenderCacheError::new(RenderCacheErrorKind::LeaseFenced));
        };
        let Some(token) = self.store.mint_token(lease.key(), lease_id).await? else {
            return Err(RenderCacheError::new(RenderCacheErrorKind::LeaseFenced));
        };
        Ok(PublicationFence {
            epoch: lease.epoch(),
            // The observed generation set is the publisher's to digest; the
            // coordinator only fences.
            generation_digest: [0; 32],
            token,
        })
    }

    async fn release(&self, lease: RebuildLease) -> Result<(), RenderCacheError> {
        let released = match lease.distributed_lease_id() {
            Some(lease_id) => self.store.release(lease.key(), lease_id).await,
            None => Ok(()),
        };
        // The local release runs even when the store failed, so in-process
        // waiters are never stranded behind a leader that has finished.
        let local = self.local.release(lease).await;
        released.and(local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ClockError;
    use crate::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use crate::identity::{KeyId, UnixMillis};

    struct FixedClock {
        now_ms: AtomicU64,
    }

    impl FixedClock {
        fn new(now_ms: u64) -> Self {
            Self {
                now_ms: AtomicU64::new(now_ms),
            }
        }

        fn set(&self, now_ms: u64) {
            self.now_ms.store(now_ms, Ordering::SeqCst);
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            Ok(UnixMillis::new(self.now_ms.load(Ordering::SeqCst)))
        }
    }

    struct UnreadableClock;

    impl Clock for UnreadableClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            Err(ClockError::timestamp_overflow())
        }
    }

    fn keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("lease-test").expect("key id"),
            RootKey::new(vec![7; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    fn key(pattern: &str) -> RenderKey {
        RenderKey::for_test(&keys(), pattern)
    }

    fn acquired(attempt: LeaseAttempt) -> u64 {
        match attempt {
            LeaseAttempt::Acquired { lease_id, .. } => lease_id,
            LeaseAttempt::Held => panic!("the key was expected to be acquirable"),
        }
    }

    #[tokio::test]
    async fn an_expired_lease_is_taken_over_and_the_former_holder_mints_nothing() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryLeaseStore::new(clock.clone());
        let key = key("/a");
        let first = acquired(store.try_acquire(&key, 1, 1_000).await.expect("acquire"));
        assert_eq!(store.mint_token(&key, first).await.expect("mint"), Some(1));

        clock.set(1_000);
        assert_eq!(
            store.mint_token(&key, first).await.expect("mint"),
            None,
            "expiry is decided by store time"
        );
        let second = acquired(store.try_acquire(&key, 1, 1_000).await.expect("acquire"));
        assert_ne!(second, first);
        assert_eq!(
            store.mint_token(&key, second).await.expect("mint"),
            Some(2),
            "a takeover continues the key's tokens"
        );
        assert_eq!(
            store.mint_token(&key, first).await.expect("mint"),
            None,
            "the former holder is fenced out for good"
        );
    }

    #[tokio::test]
    async fn releasing_a_lease_someone_else_holds_changes_nothing() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryLeaseStore::new(clock);
        let key = key("/a");
        let holder = acquired(store.try_acquire(&key, 1, 1_000).await.expect("acquire"));

        store
            .release(&key, holder + 1)
            .await
            .expect("a stale release is not an error");
        assert_eq!(
            store.try_acquire(&key, 1, 1_000).await.expect("attempt"),
            LeaseAttempt::Held,
            "only the holder's own release frees the key"
        );
        assert_eq!(
            store.mint_token(&key, holder).await.expect("mint"),
            Some(1),
            "the holder still leads"
        );
    }

    #[tokio::test]
    async fn a_takeover_records_the_epoch_of_the_holder_that_took_the_key() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryLeaseStore::new(clock.clone());
        let key = key("/a");
        assert_eq!(store.epoch(&key), None, "the key has never been leased");

        acquired(store.try_acquire(&key, 3, 1_000).await.expect("acquire"));
        assert_eq!(store.epoch(&key), Some(3));
        assert_eq!(
            store.try_acquire(&key, 9, 1_000).await.expect("attempt"),
            LeaseAttempt::Held,
            "a newer epoch does not preempt an unexpired lease"
        );
        assert_eq!(
            store.epoch(&key),
            Some(3),
            "the refused attempt recorded nothing"
        );

        clock.set(1_000);
        acquired(store.try_acquire(&key, 9, 1_000).await.expect("acquire"));
        assert_eq!(
            store.epoch(&key),
            Some(9),
            "the tenure that took the key over owns the recorded epoch"
        );
    }

    #[tokio::test]
    async fn leases_are_per_key() {
        let clock = Arc::new(FixedClock::new(0));
        let store = MemoryLeaseStore::new(clock);
        let first = key("/a");
        let second = key("/b");
        let first_lease = acquired(store.try_acquire(&first, 1, 1_000).await.expect("acquire"));
        let second_lease = acquired(store.try_acquire(&second, 1, 1_000).await.expect("acquire"));

        assert_ne!(first_lease, second_lease);
        assert_eq!(
            store.mint_token(&first, first_lease).await.expect("mint"),
            Some(1)
        );
        assert_eq!(
            store.mint_token(&second, second_lease).await.expect("mint"),
            Some(1),
            "each key counts its own tokens"
        );
        assert_eq!(
            store.mint_token(&first, second_lease).await.expect("mint"),
            None,
            "a lease is valid only for the key it was taken on"
        );
    }

    #[tokio::test]
    async fn a_store_clock_that_cannot_be_read_is_a_provider_failure() {
        let store = MemoryLeaseStore::new(Arc::new(UnreadableClock));
        let key = key("/a");

        assert_eq!(
            store
                .try_acquire(&key, 1, 1_000)
                .await
                .expect_err("no expiry decision is possible")
                .kind(),
            RenderCacheErrorKind::ProviderUnavailable
        );
        assert_eq!(
            store
                .mint_token(&key, 1)
                .await
                .expect_err("no expiry decision is possible")
                .kind(),
            RenderCacheErrorKind::ProviderUnavailable
        );
    }

    #[tokio::test]
    async fn a_store_failure_during_admission_frees_the_local_lease() {
        struct FailingStore;

        #[async_trait]
        impl LeaseStore for FailingStore {
            async fn try_acquire(
                &self,
                _key: &RenderKey,
                _epoch: u64,
                _ttl_ms: u64,
            ) -> Result<LeaseAttempt, RenderCacheError> {
                Err(RenderCacheError::new(
                    RenderCacheErrorKind::ProviderUnavailable,
                ))
            }

            async fn mint_token(
                &self,
                _key: &RenderKey,
                _lease_id: u64,
            ) -> Result<Option<u64>, RenderCacheError> {
                Ok(None)
            }

            async fn release(
                &self,
                _key: &RenderKey,
                _lease_id: u64,
            ) -> Result<(), RenderCacheError> {
                Ok(())
            }
        }

        let coordinator = FencedLeaseCoordinator::new(
            Arc::new(FailingStore),
            LocalCoordinatorLimits {
                lease_ms: 1_000,
                max_waiters: 4,
            },
        );
        let key = key("/a");

        assert_eq!(
            coordinator
                .admit(&key, 1, 0)
                .await
                .expect_err("the store decides leadership")
                .kind(),
            RenderCacheErrorKind::ProviderUnavailable
        );
        assert!(
            matches!(
                coordinator.admit(&key, 1, 1).await,
                Err(error) if error.kind() == RenderCacheErrorKind::ProviderUnavailable
            ),
            "the failed admission left no local lease behind to park waiters on"
        );
    }

    #[tokio::test]
    async fn the_hand_back_on_a_failed_admission_is_propagated_and_not_discarded() {
        // Both arms that hand the local lease back do it with `?`, so a
        // failure of the hand-back itself is admission's answer rather than
        // something swallowed on the way out. What that failure would be is
        // unreachable in this build - `LocalRebuildCoordinator::release`
        // recovers a poisoned lock and returns `Ok(())` unconditionally, and
        // the field it releases through is that concrete type rather than a
        // port a test could stand a failing double in front of. So what is
        // proved here is the reachable half: when the hand-back succeeds,
        // the store's own failure is the one reported, with its kind intact,
        // and the released lease really is free afterwards.
        //
        // The store answers a kind nothing else in this path answers, so the
        // assertions below are about which failure surfaced rather than
        // merely that one did.
        struct RefusingStore;

        #[async_trait]
        impl LeaseStore for RefusingStore {
            async fn try_acquire(
                &self,
                _key: &RenderKey,
                _epoch: u64,
                _ttl_ms: u64,
            ) -> Result<LeaseAttempt, RenderCacheError> {
                Err(RenderCacheError::new(RenderCacheErrorKind::KeyInvalid))
            }

            async fn mint_token(
                &self,
                _key: &RenderKey,
                _lease_id: u64,
            ) -> Result<Option<u64>, RenderCacheError> {
                Ok(None)
            }

            async fn release(
                &self,
                _key: &RenderKey,
                _lease_id: u64,
            ) -> Result<(), RenderCacheError> {
                Ok(())
            }
        }

        let limits = LocalCoordinatorLimits {
            lease_ms: 1_000,
            max_waiters: 4,
        };
        let coordinator = FencedLeaseCoordinator::new(Arc::new(RefusingStore), limits);
        let key = key("/a");

        let failure = coordinator
            .admit(&key, 1, 0)
            .await
            .expect_err("the store decides leadership");
        assert_eq!(
            failure.kind(),
            RenderCacheErrorKind::KeyInvalid,
            "a successful hand-back reports the store's own failure, not its own success"
        );

        // The local coordinator really did take the lease back: a fresh
        // admission at the same instant reaches the store again rather than
        // parking behind the request that failed.
        let again = coordinator
            .admit(&key, 1, 0)
            .await
            .expect_err("the store still refuses");
        assert_eq!(again.kind(), RenderCacheErrorKind::KeyInvalid);
    }
}
