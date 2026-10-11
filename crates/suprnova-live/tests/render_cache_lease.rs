//! Cross-node rebuild leadership over a lease store: one leader per key, no
//! cross-node waiting, expiry and token minting decided by store time alone,
//! and in-process waiters still parked on the local leader.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::RenderCacheErrorKind;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::lease::{
    FencedLeaseCoordinator, LeaseAttempt, LeaseStore, MemoryLeaseStore,
};
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};

/// Store clock under the test's control: every cross-node expiry decision is
/// taken against this value, never against a node's `now_ms` argument.
#[derive(Debug)]
struct ControlledClock {
    now: AtomicU64,
}

impl ControlledClock {
    fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl Clock for ControlledClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
    }
}

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn key(path: &str) -> RenderKey {
    RenderKey::for_test(&keys_from(6), path)
}

fn limits(lease_ms: u64) -> LocalCoordinatorLimits {
    LocalCoordinatorLimits {
        lease_ms,
        max_waiters: 4,
    }
}

#[tokio::test]
async fn two_coordinators_over_one_store_admit_one_leader_and_one_bypass() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = Arc::new(MemoryLeaseStore::new(clock.clone()));
    let first = FencedLeaseCoordinator::new(Arc::clone(&store), limits(5_000));
    let second = FencedLeaseCoordinator::new(Arc::clone(&store), limits(5_000));
    let key = key("/a");

    let RebuildAdmission::Lead(lease) = first.admit(&key, 1, 1_000).await.expect("admit") else {
        panic!("the first admission leads")
    };
    assert!(
        lease.distributed_lease_id().is_some(),
        "a leader carries the store's lease id"
    );
    assert!(
        matches!(
            second.admit(&key, 1, 1_000).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "another node never waits on this one; it renders without publishing"
    );

    let fence = first
        .publish_token(&lease, 1_100)
        .await
        .expect("the leader mints a token");
    assert_eq!(fence.epoch, 1);
    assert_eq!(fence.token, 1);
    assert_eq!(
        fence.generation_digest, [0; 32],
        "the coordinator never invents a digest; the publisher fills it in"
    );
    assert_eq!(
        first
            .publish_token(&lease, 1_200)
            .await
            .expect("token")
            .token,
        2,
        "store tokens are monotonic per key"
    );
}

#[tokio::test]
async fn a_dead_leaders_lease_expires_by_store_time_and_the_takeover_is_fenced() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = Arc::new(MemoryLeaseStore::new(clock.clone()));
    let dead = FencedLeaseCoordinator::new(Arc::clone(&store), limits(1_000));
    let taking_over = FencedLeaseCoordinator::new(Arc::clone(&store), limits(1_000));
    let key = key("/a");

    let RebuildAdmission::Lead(dead_lease) = dead.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the first admission leads")
    };
    let dead_fence = dead
        .publish_token(&dead_lease, 10)
        .await
        .expect("the leader mints a token before dying");

    clock.set(1_000);

    let RebuildAdmission::Lead(next_lease) = taking_over.admit(&key, 1, 10).await.expect("admit")
    else {
        panic!("an expired lease is taken over")
    };
    assert_eq!(
        dead.publish_token(&dead_lease, 20)
            .await
            .expect_err("the former leader lost its lease")
            .kind(),
        RenderCacheErrorKind::LeaseFenced
    );
    let next_fence = taking_over
        .publish_token(&next_lease, 20)
        .await
        .expect("the new leader mints a token");
    assert!(
        next_fence.token > dead_fence.token,
        "a takeover's tokens continue above the former leader's"
    );
}

#[tokio::test]
async fn node_clock_skew_does_not_extend_a_lease() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = Arc::new(MemoryLeaseStore::new(clock.clone()));
    let skewed = FencedLeaseCoordinator::new(Arc::clone(&store), limits(1_000));
    let other = FencedLeaseCoordinator::new(Arc::clone(&store), limits(1_000));
    let key = key("/a");

    // This node's clock runs far ahead of store time, so nothing it reports
    // about its own lease horizon lines up with the store's.
    let RebuildAdmission::Lead(lease) = skewed.admit(&key, 1, 10_000_000).await.expect("admit")
    else {
        panic!("the first admission leads")
    };
    assert!(
        skewed.publish_token(&lease, 99_999_999).await.is_ok(),
        "a node time past this node's own lease horizon does not fence a lease the store still holds"
    );

    clock.set(1_000);

    assert!(
        matches!(
            other.admit(&key, 1, 10_000_100).await.expect("admit"),
            RebuildAdmission::Lead(_)
        ),
        "the store expired the lease on its own clock"
    );
    assert_eq!(
        skewed
            .publish_token(&lease, 10_000_100)
            .await
            .expect_err("the skewed node cannot extend its lease")
            .kind(),
        RenderCacheErrorKind::LeaseFenced
    );
}

#[tokio::test]
async fn in_process_waiters_still_wait_on_the_local_leader() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = Arc::new(MemoryLeaseStore::new(clock.clone()));
    let coordinator = FencedLeaseCoordinator::new(store, limits(5_000));
    let key = key("/a");

    let RebuildAdmission::Lead(lease) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!("the first admission leads")
    };
    let RebuildAdmission::Wait(wait) = coordinator.admit(&key, 1, 1_001).await.expect("admit")
    else {
        panic!("a second request on the same node waits on its leader")
    };

    // `started_rx.await` only resolves once the spawned task has run past
    // `started_tx.send`, and nothing but `wait.wait().await` separates that
    // send from the waiter's first poll, so by the time this task observes
    // `started_rx` resolve, the spawned task has already registered its waker
    // and parked. This is an observed-state barrier, not a timing assumption.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        started_tx.send(()).expect("send started signal");
        wait.wait().await;
    });
    started_rx.await.expect("waiter task started");
    assert!(
        !handle.is_finished(),
        "the waiter must not resolve while the leader holds the lease"
    );

    coordinator.release(*lease).await.expect("release");
    handle
        .await
        .expect("the waiter resolves once the leader releases");
}

#[tokio::test]
async fn release_frees_the_store_lease_for_the_next_node() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = Arc::new(MemoryLeaseStore::new(clock.clone()));
    let first = FencedLeaseCoordinator::new(Arc::clone(&store), limits(5_000));
    let second = FencedLeaseCoordinator::new(Arc::clone(&store), limits(5_000));
    let key = key("/a");

    let RebuildAdmission::Lead(lease) = first.admit(&key, 1, 1_000).await.expect("admit") else {
        panic!("the first admission leads")
    };
    let released_fence = first.publish_token(&lease, 1_100).await.expect("token");
    first.release(*lease).await.expect("release");

    let RebuildAdmission::Lead(next) = second.admit(&key, 1, 1_200).await.expect("admit") else {
        panic!("a released lease is available to the next node")
    };
    assert!(
        second
            .publish_token(&next, 1_300)
            .await
            .expect("token")
            .token
            > released_fence.token,
        "tokens never restart after a release"
    );
}

#[tokio::test]
async fn the_lease_store_holds_a_key_until_release_and_then_stops_minting() {
    let clock = Arc::new(ControlledClock::new(0));
    let store = MemoryLeaseStore::new(clock.clone());
    let key = key("/a");

    let LeaseAttempt::Acquired {
        lease_id,
        expires_at_ms,
    } = store.try_acquire(&key, 1, 1_000).await.expect("acquire")
    else {
        panic!("an unheld key is acquirable")
    };
    assert_eq!(expires_at_ms, 1_000, "expiry is store time plus the ttl");
    assert_eq!(
        store.try_acquire(&key, 1, 1_000).await.expect("attempt"),
        LeaseAttempt::Held,
        "an unexpired lease holds the key"
    );
    assert_eq!(
        store.mint_token(&key, lease_id).await.expect("mint"),
        Some(1)
    );

    store.release(&key, lease_id).await.expect("release");
    assert_eq!(
        store.mint_token(&key, lease_id).await.expect("mint"),
        None,
        "a released lease mints nothing"
    );

    let LeaseAttempt::Acquired {
        lease_id: next_lease_id,
        ..
    } = store.try_acquire(&key, 1, 1_000).await.expect("acquire")
    else {
        panic!("a released key is acquirable again")
    };
    assert_ne!(next_lease_id, lease_id, "lease ids are never reused");
    assert_eq!(
        store.mint_token(&key, next_lease_id).await.expect("mint"),
        Some(2),
        "tokens continue above every token the key has minted"
    );
}
