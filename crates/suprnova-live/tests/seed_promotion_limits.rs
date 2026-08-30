//! Seed-promotion abuse and storage bound tests.

mod promotion_support;

use promotion_support::{context, harness, nonce, promotion_limits, signed_seed};
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{LiveInstanceLedger, PromotionRecord};
use suprnova_live::promotion::{PromotionErrorKind, PromotionLimitConfig, PromotionLimits};

fn configured(
    rate: usize,
    outstanding: usize,
    route_component: usize,
    reservations: usize,
    buckets: usize,
    max_seed_bytes: usize,
) -> PromotionLimits {
    PromotionLimits::new(PromotionLimitConfig {
        max_seed_bytes,
        window_ms: 1_000,
        max_promotions_per_window: rate,
        max_outstanding_per_scope: outstanding,
        max_outstanding_per_route_component: route_component,
        promotion_lease_ms: 100,
        abandoned_retention_ms: 200,
        instance_lifetime_ms: 1_000,
        max_reservations: reservations,
        max_rate_buckets: buckets,
    })
    .expect("promotion limits are valid")
}

#[tokio::test]
async fn per_window_rate_is_bounded_without_touching_the_ledger_for_rejection() {
    let harness = harness(configured(2, 8, 8, 64, 32, 4_096), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let context = context(0xa0);
    for start in [0x20, 0x21] {
        harness
            .service
            .promote(&seed, nonce(start), &context)
            .await
            .expect("promotion within rate succeeds");
    }
    let error = harness
        .service
        .promote(&seed, nonce(0x22), &context)
        .await
        .expect_err("third promotion is rate limited");
    assert_eq!(error.kind(), PromotionErrorKind::RateLimited);
}

#[tokio::test]
async fn outstanding_and_route_component_limits_are_distinct() {
    let outstanding = harness(configured(8, 1, 8, 64, 32, 4_096), 64);
    let seed = signed_seed(&outstanding.keys, "rust");
    let outstanding_context = context(0xa1);
    outstanding
        .service
        .promote(&seed, nonce(0x23), &outstanding_context)
        .await
        .expect("first promotion succeeds");
    assert_eq!(
        outstanding
            .service
            .promote(&seed, nonce(0x24), &outstanding_context)
            .await
            .expect_err("scope outstanding limit applies")
            .kind(),
        PromotionErrorKind::OutstandingLimit
    );

    let route = harness(configured(8, 8, 1, 64, 32, 4_096), 64);
    let seed = signed_seed(&route.keys, "rust");
    let context = context(0xa2);
    route
        .service
        .promote(&seed, nonce(0x25), &context)
        .await
        .expect("first promotion succeeds");
    assert_eq!(
        route
            .service
            .promote(&seed, nonce(0x26), &context)
            .await
            .expect_err("route/component outstanding limit applies")
            .kind(),
        PromotionErrorKind::RouteComponentLimit
    );
}

#[tokio::test]
async fn seed_bytes_reservations_and_rate_bucket_cardinality_are_bounded() {
    let input = harness(configured(8, 8, 8, 64, 32, 64), 64);
    let oversized = vec![b' '; 65];
    assert_eq!(
        input
            .service
            .promote(&oversized, nonce(0x27), &context(0xa3))
            .await
            .expect_err("service preflights byte limit")
            .kind(),
        PromotionErrorKind::InputTooLarge
    );
    assert_eq!(input.generator.calls(), 0);

    let storage = harness(configured(8, 8, 8, 1, 32, 4_096), 64);
    let seed = signed_seed(&storage.keys, "rust");
    storage
        .service
        .promote(&seed, nonce(0x28), &context(0xa4))
        .await
        .expect("first reservation succeeds");
    assert_eq!(
        storage
            .service
            .promote(&seed, nonce(0x29), &context(0xa4))
            .await
            .expect_err("reservation storage is bounded")
            .kind(),
        PromotionErrorKind::StorageLimit
    );

    let buckets = harness(configured(8, 8, 8, 64, 1, 4_096), 64);
    let seed = signed_seed(&buckets.keys, "rust");
    buckets
        .service
        .promote(&seed, nonce(0x2a), &context(0xa5))
        .await
        .expect("first rate bucket succeeds");
    assert_eq!(
        buckets
            .service
            .promote(&seed, nonce(0x2b), &context(0xa6))
            .await
            .expect_err("rate bucket storage is bounded")
            .kind(),
        PromotionErrorKind::StorageLimit
    );
}

#[tokio::test]
async fn ledger_failure_retains_a_bounded_abandoned_nonce_without_partial_instance() {
    let harness = harness(promotion_limits(), 1);
    harness
        .ledger
        .promote(PromotionRecord::new(
            promotion_support::scope(0xff),
            promotion_support::instance(0xee),
            promotion_support::idempotency(0xdd),
            promotion_support::digest(0xcc),
            Revision::new(0),
            UnixMillis::new(5_000),
        ))
        .await
        .expect("preload fills ledger capacity");
    let seed = signed_seed(&harness.keys, "rust");
    let context = context(0xa7);
    let failed_nonce = nonce(0x2c);

    assert_eq!(
        harness
            .service
            .promote(&seed, failed_nonce.clone(), &context)
            .await
            .expect_err("ledger failure is classified")
            .kind(),
        PromotionErrorKind::LedgerUnavailable
    );
    assert_eq!(
        harness
            .service
            .promote(&seed, failed_nonce, &context)
            .await
            .expect_err("abandoned reservation is retained")
            .kind(),
        PromotionErrorKind::AbandonedRetention
    );
    assert!(
        harness
            .ledger
            .inspect(
                &promotion_support::scope(0xa7),
                &promotion_support::instance(0xd0),
            )
            .expect("inspection succeeds")
            .is_none()
    );

    harness.clock.set(1_301);
    assert_eq!(
        harness
            .service
            .promote(&seed, nonce(0x2c), &context)
            .await
            .expect_err("expired abandoned retention admits a fresh bounded attempt")
            .kind(),
        PromotionErrorKind::LedgerUnavailable
    );
}

#[test]
fn promotion_limits_reject_zero_and_unbounded_values() {
    let mut config = PromotionLimitConfig {
        max_seed_bytes: 4_096,
        window_ms: 1_000,
        max_promotions_per_window: 8,
        max_outstanding_per_scope: 8,
        max_outstanding_per_route_component: 4,
        promotion_lease_ms: 100,
        abandoned_retention_ms: 200,
        instance_lifetime_ms: 1_000,
        max_reservations: 64,
        max_rate_buckets: 32,
    };
    config.max_promotions_per_window = 0;
    assert_eq!(
        PromotionLimits::new(config)
            .expect_err("zero rate is invalid")
            .kind(),
        PromotionErrorKind::InvalidConfiguration
    );
}
