//! Deterministic concurrent public-seed promotion tests.

mod promotion_support;

use std::sync::Arc;

use promotion_support::{context, harness, nonce, promotion_limits, signed_seed};
use suprnova_live::promotion::PromotionErrorKind;
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_exact_nonce_replay_never_creates_two_instances() {
    let harness = Arc::new(harness(promotion_limits(), 64));
    let seed = Arc::new(signed_seed(&harness.keys, "rust"));
    let context = Arc::new(context(0xb0));
    let nonce = nonce(0x30);
    let barrier = Arc::new(Barrier::new(3));
    let mut tasks = Vec::new();

    for _ in 0..2 {
        let harness = harness.clone();
        let seed = seed.clone();
        let context = context.clone();
        let nonce = nonce.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            harness.service.promote(&seed, nonce, &context).await
        }));
    }

    barrier.wait().await;
    let mut successful_ids = Vec::new();
    let mut in_progress = 0;
    for task in tasks {
        match task.await.expect("promotion task joins") {
            Ok(promoted) => successful_ids.push(promoted.instance_id().clone()),
            Err(error) if error.kind() == PromotionErrorKind::InProgress => in_progress += 1,
            Err(error) => panic!("unexpected promotion error: {error:?}"),
        }
    }
    successful_ids.dedup();
    assert_eq!(successful_ids.len(), 1);
    assert!(successful_ids.len() + in_progress >= 1);

    let recovered = harness
        .service
        .promote(&seed, nonce, &context)
        .await
        .expect("post-race exact retry recovers authority");
    assert_eq!(recovered.instance_id(), &successful_ids[0]);
}
