//! Tier 0 ledger expiry and consumed-authority tests.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{ManualClock, digest, idempotency, instance, ledger, promote_default, scope};
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{
    ClaimOutcome, ClaimRequest, LedgerLimits, LedgerPhase, LiveInstanceLedger,
    MemoryInstanceLedger, PromotionOutcome, PromotionRecord, RefreshReason,
};

fn request(
    scope: suprnova_live::identity::ScopeFingerprint,
    instance: suprnova_live::identity::InstanceId,
    base: u64,
) -> ClaimRequest {
    ClaimRequest::new(
        scope,
        instance,
        Revision::new(base),
        idempotency(0xa0),
        digest(0xb0),
    )
}

#[tokio::test]
async fn abandoned_claim_consumes_authority_without_rolling_revision_back() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x31);
    let instance = instance(0x41);
    promote_default(&ledger, scope.clone(), instance.clone()).await;
    let grant = match ledger
        .claim(request(scope.clone(), instance.clone(), 0))
        .await
        .expect("claim succeeds")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected grant, got {other:?}"),
    };

    ledger
        .abandon(&grant.into_token())
        .await
        .expect("matching claim abandons");

    for base in [0, 1] {
        assert!(matches!(
            ledger
                .claim(request(scope.clone(), instance.clone(), base))
                .await
                .expect("consumed authority classifies"),
            ClaimOutcome::RefreshRequired(RefreshReason::Consumed)
        ));
    }
    let inspection = ledger
        .inspect(&scope, &instance)
        .expect("inspection succeeds")
        .expect("record remains until instance expiry");
    assert_eq!(inspection.current_revision(), Revision::new(1));
    assert_eq!(inspection.phase(), LedgerPhase::Consumed);
}

#[tokio::test]
async fn dropped_execution_claim_releases_authority_for_an_exact_retry() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x35);
    let instance = instance(0x45);
    promote_default(&ledger, scope.clone(), instance.clone()).await;
    let grant = match ledger
        .claim(request(scope.clone(), instance.clone(), 0))
        .await
        .expect("claim succeeds")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected grant, got {other:?}"),
    };

    ledger.abandon_on_drop(grant.into_token());

    let retry = match ledger
        .claim(request(scope.clone(), instance.clone(), 0))
        .await
        .expect("released claim accepts an exact retry")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected retry grant, got {other:?}"),
    };
    let inspection = ledger
        .inspect(&scope, &instance)
        .expect("inspection succeeds")
        .expect("instance retained");
    assert_eq!(inspection.current_revision(), Revision::new(1));
    assert_eq!(inspection.phase(), LedgerPhase::Pending);

    ledger
        .abandon(&retry.into_token())
        .await
        .expect("test cleanup consumes the retry");
}

#[tokio::test]
async fn expired_claim_lease_becomes_terminal_and_cannot_be_reclaimed() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock.clone(), 2);
    let scope = scope(0x32);
    let instance = instance(0x42);
    promote_default(&ledger, scope.clone(), instance.clone()).await;
    let grant = match ledger
        .claim(request(scope.clone(), instance.clone(), 0))
        .await
        .expect("claim succeeds")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected grant, got {other:?}"),
    };

    clock.set(1_101);
    assert!(matches!(
        ledger
            .claim(request(scope.clone(), instance.clone(), 1))
            .await
            .expect("expired lease classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::ClaimExpired)
    ));
    assert_eq!(
        ledger
            .commit(
                &grant.into_token(),
                suprnova_live::ledger::AcceptedOutcome::new(
                    suprnova_live::ledger::AcceptedOutcomeKind::NoRender,
                    digest(0xc0),
                ),
            )
            .await
            .expect_err("expired token cannot commit")
            .kind(),
        suprnova_live::ledger::LedgerErrorKind::ClaimExpired
    );
}

#[tokio::test]
async fn missing_and_expired_instances_require_fresh_rendering() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock.clone(), 2);
    let scope = scope(0x33);
    let instance = instance(0x43);

    assert!(matches!(
        ledger
            .claim(request(scope.clone(), instance.clone(), 0))
            .await
            .expect("missing instance classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::Missing)
    ));

    promote_default(&ledger, scope.clone(), instance.clone()).await;
    clock.set(5_000);
    assert!(matches!(
        ledger
            .claim(request(scope.clone(), instance.clone(), 0))
            .await
            .expect("expired instance classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::InstanceExpired)
    ));
    assert!(
        ledger
            .inspect(&scope, &instance)
            .expect("inspection succeeds")
            .is_none()
    );
}

#[tokio::test]
async fn requested_retry_key_expires_even_behind_the_global_cleanup_budget() {
    let clock = Arc::new(ManualClock::new(100));
    let ledger = MemoryInstanceLedger::new(
        clock.clone(),
        LedgerLimits::new(100, 1_000, 2, 100).expect("ledger limits are valid"),
    );
    for start in 0..66_u8 {
        assert!(matches!(
            ledger
                .promote(PromotionRecord::new(
                    scope(start),
                    instance(start),
                    idempotency(start),
                    digest(start.wrapping_add(1)),
                    Revision::new(0),
                    UnixMillis::new(200),
                ))
                .await
                .expect("setup promotion succeeds"),
            PromotionOutcome::Created(_)
        ));
    }

    clock.set(201);
    let replacement = ledger
        .promote(PromotionRecord::new(
            scope(0),
            instance(0xf0),
            idempotency(0),
            digest(1),
            Revision::new(0),
            UnixMillis::new(300),
        ))
        .await
        .expect("expired retry key is reusable");
    let PromotionOutcome::Created(authority) = replacement else {
        panic!("expired retry metadata must not return an old authority");
    };
    assert_eq!(authority.instance_id(), &instance(0xf0));
}
