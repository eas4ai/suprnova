//! Deterministic Tier 0 ledger concurrency tests.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{ManualClock, digest, idempotency, instance, ledger, promote_default, scope};
use suprnova_live::identity::Revision;
use suprnova_live::ledger::{ClaimOutcome, ClaimRequest, LiveInstanceLedger};
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_claims_for_one_base_revision_grant_exactly_one_token() {
    let (granted, in_progress) = race_once(0).await;

    assert_eq!(granted, 1);
    assert_eq!(in_progress, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_claim_races_preserve_the_single_grant_invariant() {
    for seed in 1_u8..=128 {
        let (granted, in_progress) = race_once(seed).await;
        assert_eq!(granted, 1);
        assert_eq!(in_progress, 1);
    }
}

async fn race_once(seed: u8) -> (usize, usize) {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x30_u8.wrapping_add(seed));
    let instance = instance(0x40_u8.wrapping_add(seed));
    promote_default(&ledger, scope.clone(), instance.clone()).await;
    let barrier = Arc::new(Barrier::new(3));
    let request = ClaimRequest::new(
        scope,
        instance,
        Revision::new(0),
        idempotency(0x50_u8.wrapping_add(seed)),
        digest(0x60_u8.wrapping_add(seed)),
    );

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let ledger = ledger.clone();
        let barrier = barrier.clone();
        let request = request.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            ledger.claim(request).await.expect("claim classifies")
        }));
    }

    barrier.wait().await;
    let mut granted = 0;
    let mut in_progress = 0;
    for task in tasks {
        match task.await.expect("claim task joins") {
            ClaimOutcome::Granted(_) => granted += 1,
            ClaimOutcome::InProgress { .. } => in_progress += 1,
            other => panic!("unexpected concurrent outcome: {other:?}"),
        }
    }

    (granted, in_progress)
}
