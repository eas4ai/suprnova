//! One accepted publication per key and fence; bounded waiters; expiry and
//! cancellation release the lease; fencing is monotonic.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, LocalRebuildCoordinator, RebuildAdmission, RebuildCoordinator,
};

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

#[tokio::test]
async fn one_leader_per_key_and_fence_with_bounded_waiters() {
    let coordinator = LocalRebuildCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 5_000,
        max_waiters: 2,
    });
    let key = key("/a");
    let RebuildAdmission::Lead(lease) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!("first admission leads")
    };
    assert!(matches!(
        coordinator.admit(&key, 1, 1_001).await.expect("admit"),
        RebuildAdmission::Wait(_)
    ));
    assert!(matches!(
        coordinator.admit(&key, 1, 1_002).await.expect("admit"),
        RebuildAdmission::Wait(_)
    ));
    assert!(
        matches!(
            coordinator.admit(&key, 1, 1_003).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "waiters are bounded"
    );
    let fence = coordinator
        .publish_token(&lease, 1_500)
        .await
        .expect("token");
    assert_eq!(fence.epoch, 1);
    assert!(fence.token > 0);
    coordinator.release(*lease).await.expect("release");
    assert!(
        matches!(
            coordinator.admit(&key, 1, 1_600).await.expect("admit"),
            RebuildAdmission::Lead(_)
        ),
        "release frees the key"
    );
}

#[tokio::test]
async fn an_expired_or_cancelled_lease_cannot_publish_and_a_newer_leader_wins() {
    let coordinator = LocalRebuildCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 100,
        max_waiters: 4,
    });
    let key = key("/a");
    let RebuildAdmission::Lead(old) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!()
    };
    let RebuildAdmission::Lead(new) = coordinator
        .admit(&key, 1, 1_200)
        .await
        .expect("admit after expiry")
    else {
        panic!("expiry frees the lease")
    };
    assert!(
        coordinator.publish_token(&old, 1_250).await.is_err(),
        "an expired former leader is fenced out"
    );
    let fence = coordinator
        .publish_token(&new, 1_250)
        .await
        .expect("current leader publishes");
    coordinator.release(*new).await.expect("release");
    let RebuildAdmission::Lead(cancelled) = coordinator.admit(&key, 1, 1_300).await.expect("admit")
    else {
        panic!()
    };
    coordinator
        .release(*cancelled)
        .await
        .expect("cancellation releases without publishing");
    let RebuildAdmission::Lead(next) = coordinator.admit(&key, 1, 1_400).await.expect("admit")
    else {
        panic!()
    };
    let later = coordinator
        .publish_token(&next, 1_450)
        .await
        .expect("token");
    assert!(
        later.token > fence.token,
        "publication tokens are monotonic"
    );
}

#[tokio::test]
async fn a_new_epoch_supersedes_an_older_leader() {
    let coordinator = LocalRebuildCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 5_000,
        max_waiters: 4,
    });
    let key = key("/a");
    let RebuildAdmission::Lead(old_epoch) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!()
    };
    let RebuildAdmission::Lead(new_epoch) = coordinator.admit(&key, 2, 1_001).await.expect("admit")
    else {
        panic!("a newer epoch leads")
    };
    assert!(coordinator.publish_token(&old_epoch, 1_002).await.is_err());
    assert_eq!(
        coordinator
            .publish_token(&new_epoch, 1_002)
            .await
            .expect("token")
            .epoch,
        2
    );
}

#[tokio::test]
async fn a_waiter_resolves_only_after_the_leader_releases() {
    let coordinator = LocalRebuildCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 5_000,
        max_waiters: 4,
    });
    let key = key("/a");
    let RebuildAdmission::Lead(lease) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!("first admission leads")
    };
    let RebuildAdmission::Wait(wait) = coordinator.admit(&key, 1, 1_001).await.expect("admit")
    else {
        panic!("second admission waits")
    };

    // `started_rx.await` only resolves once the spawned task has run past
    // `started_tx.send`, and nothing but `wait.wait().await` separates that
    // send from the waiter's first poll, so by the time this task observes
    // `started_rx` resolve, the spawned task has already registered its
    // waker under the completion lock and parked. This gives us an
    // observed-state barrier instead of a timing assumption.
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
async fn a_dropped_waiter_does_not_block_completion_or_panic() {
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    let coordinator = LocalRebuildCoordinator::new(LocalCoordinatorLimits {
        lease_ms: 5_000,
        max_waiters: 4,
    });
    let key = key("/a");
    let RebuildAdmission::Lead(lease) = coordinator.admit(&key, 1, 1_000).await.expect("admit")
    else {
        panic!("first admission leads")
    };
    let RebuildAdmission::Wait(wait) = coordinator.admit(&key, 1, 1_001).await.expect("admit")
    else {
        panic!("second admission waits")
    };

    // Poll exactly once to register the waiter's waker under the completion
    // lock, then drop the waiter without ever resolving it: this models a
    // cancelled request. The leftover, stale waker in the slot must not
    // block or panic the leader's own completion.
    let mut wait = Box::pin(wait);
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(wait.as_mut().poll(&mut context), Poll::Pending));
    drop(wait);

    coordinator
        .release(*lease)
        .await
        .expect("release completes even though a waiter's waker is stale");
    assert!(
        matches!(
            coordinator.admit(&key, 1, 1_600).await.expect("admit"),
            RebuildAdmission::Lead(_)
        ),
        "release frees the key even with a dropped waiter registered"
    );
}
