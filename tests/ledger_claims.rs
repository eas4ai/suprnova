//! Tier 0 instance-ledger state-machine contract tests.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{
    ManualClock, digest, idempotency, instance, ledger, promote_default, promotion, scope,
};
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LedgerPhase,
    LiveInstanceLedger, PromotionOutcome,
};

#[tokio::test]
async fn claim_advances_monotonically_and_exact_duplicates_observe_one_outcome() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x10);
    let instance = instance(0x20);
    promote_default(&ledger, scope.clone(), instance.clone()).await;

    let request = ClaimRequest::new(
        scope.clone(),
        instance.clone(),
        Revision::new(0),
        idempotency(0x60),
        digest(0x70),
    );
    let grant = match ledger.claim(request.clone()).await.expect("claim succeeds") {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected granted claim, got {other:?}"),
    };
    assert_eq!(grant.successor_revision(), Revision::new(1));

    assert!(matches!(
        ledger.claim(request.clone()).await.expect("duplicate checks"),
        ClaimOutcome::InProgress {
            successor_revision
        } if successor_revision == Revision::new(1)
    ));

    let accepted = AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x80));
    ledger
        .commit(&grant.into_token(), accepted.clone())
        .await
        .expect("matching claim commits");

    let duplicate = ledger
        .claim(request)
        .await
        .expect("accepted duplicate is readable");
    let metadata = match duplicate {
        ClaimOutcome::Accepted(metadata) => metadata,
        other => panic!("expected accepted duplicate, got {other:?}"),
    };
    assert_eq!(metadata.base_revision(), Revision::new(0));
    assert_eq!(metadata.successor_revision(), Revision::new(1));
    assert_eq!(metadata.outcome(), &accepted);

    let next = ledger
        .claim(ClaimRequest::new(
            scope,
            instance,
            Revision::new(1),
            idempotency(0x61),
            digest(0x71),
        ))
        .await
        .expect("successor revision is claimable");
    assert!(matches!(
        next,
        ClaimOutcome::Granted(ref grant) if grant.successor_revision() == Revision::new(2)
    ));
}

#[tokio::test]
async fn stale_bases_and_mismatched_idempotency_never_join_pending_or_accepted_work() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x11);
    let instance = instance(0x21);
    promote_default(&ledger, scope.clone(), instance.clone()).await;
    let original = ClaimRequest::new(
        scope.clone(),
        instance.clone(),
        Revision::new(0),
        idempotency(0x62),
        digest(0x72),
    );
    let grant = match ledger
        .claim(original.clone())
        .await
        .expect("claim succeeds")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected grant, got {other:?}"),
    };

    for mismatch in [
        ClaimRequest::new(
            scope.clone(),
            instance.clone(),
            Revision::new(0),
            idempotency(0x63),
            digest(0x72),
        ),
        ClaimRequest::new(
            scope.clone(),
            instance.clone(),
            Revision::new(0),
            idempotency(0x62),
            digest(0x73),
        ),
    ] {
        assert!(matches!(
            ledger.claim(mismatch).await.expect("mismatch classifies"),
            ClaimOutcome::IdempotencyConflict
        ));
    }

    ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x83)),
        )
        .await
        .expect("claim commits");

    let stale = ledger
        .claim(ClaimRequest::new(
            scope.clone(),
            instance.clone(),
            Revision::new(0),
            idempotency(0x64),
            digest(0x74),
        ))
        .await
        .expect("stale request classifies");
    assert!(matches!(
        stale,
        ClaimOutcome::Stale { current_revision } if current_revision == Revision::new(1)
    ));

    let accepted_mismatch = ledger
        .claim(ClaimRequest::new(
            scope,
            instance,
            Revision::new(0),
            idempotency(0x62),
            digest(0x75),
        ))
        .await
        .expect("accepted mismatch classifies");
    assert!(matches!(
        accepted_mismatch,
        ClaimOutcome::IdempotencyConflict
    ));
}

#[tokio::test]
async fn accepted_history_and_provider_inspection_are_bounded_metadata_only() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x12);
    let instance = instance(0x22);
    promote_default(&ledger, scope.clone(), instance.clone()).await;

    for base in 0_u64..3 {
        let request = ClaimRequest::new(
            scope.clone(),
            instance.clone(),
            Revision::new(base),
            idempotency(0x70 + base as u8),
            digest(0x80 + base as u8),
        );
        let grant = match ledger.claim(request).await.expect("claim succeeds") {
            ClaimOutcome::Granted(grant) => grant,
            other => panic!("expected grant, got {other:?}"),
        };
        ledger
            .commit(
                &grant.into_token(),
                AcceptedOutcome::new(AcceptedOutcomeKind::Validation, digest(0x90 + base as u8)),
            )
            .await
            .expect("claim commits");
    }

    let inspection = ledger
        .inspect(&scope, &instance)
        .expect("inspection succeeds")
        .expect("instance exists");
    assert_eq!(inspection.current_revision(), Revision::new(3));
    assert_eq!(inspection.accepted_outcome_count(), 2);
    assert_eq!(inspection.phase(), LedgerPhase::Ready);

    let pruned = ledger
        .claim(ClaimRequest::new(
            scope,
            instance,
            Revision::new(0),
            idempotency(0x70),
            digest(0x80),
        ))
        .await
        .expect("pruned duplicate classifies");
    assert!(matches!(
        pruned,
        ClaimOutcome::Stale { current_revision } if current_revision == Revision::new(3)
    ));
}

#[tokio::test]
async fn exact_promotion_retry_recovers_authority_but_changed_input_conflicts() {
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = ledger(clock, 2);
    let scope = scope(0x13);
    let original_instance = instance(0x23);
    let record = promotion(scope.clone(), original_instance.clone());

    assert!(matches!(
        ledger
            .promote(record.clone())
            .await
            .expect("promotion succeeds"),
        PromotionOutcome::Created(_)
    ));

    let retry = ledger
        .promote(record.with_instance_id(instance(0x24)))
        .await
        .expect("exact retry recovers");
    let authority = match retry {
        PromotionOutcome::Existing(authority) => authority,
        other => panic!("expected existing promotion, got {other:?}"),
    };
    assert_eq!(authority.instance_id(), &original_instance);

    let conflict = promotion(scope, instance(0x25)).with_request_digest(digest(0xf0));
    assert!(matches!(
        ledger.promote(conflict).await.expect("conflict classifies"),
        PromotionOutcome::IdempotencyConflict
    ));
}

#[tokio::test]
async fn opaque_claim_tokens_are_bound_to_the_provider_that_issued_them() {
    let clock = Arc::new(ManualClock::new(1_000));
    let first = ledger(clock.clone(), 2);
    let second = ledger(clock, 2);
    let scope = scope(0x14);
    let instance = instance(0x26);
    promote_default(&first, scope.clone(), instance.clone()).await;
    promote_default(&second, scope.clone(), instance.clone()).await;
    let request = ClaimRequest::new(
        scope,
        instance,
        Revision::new(0),
        idempotency(0x65),
        digest(0x76),
    );
    let first_grant = match first
        .claim(request.clone())
        .await
        .expect("first claim succeeds")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected first grant, got {other:?}"),
    };
    let _second_grant = match second.claim(request).await.expect("second claim succeeds") {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected second grant, got {other:?}"),
    };

    assert_eq!(
        second
            .commit(
                &first_grant.into_token(),
                AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x86)),
            )
            .await
            .expect_err("a token cannot cross providers")
            .kind(),
        suprnova_live::ledger::LedgerErrorKind::ClaimMismatch
    );
}

#[test]
fn ledger_limits_reject_zero_and_unbounded_configurations() {
    use suprnova_live::ledger::{LedgerErrorKind, LedgerLimits};

    assert_eq!(
        LedgerLimits::new(0, 10_000, 2, 64)
            .expect_err("zero lease is invalid")
            .kind(),
        LedgerErrorKind::InvalidConfiguration
    );
    assert_eq!(
        LedgerLimits::new(100, u64::MAX, 2, 64)
            .expect_err("unbounded lifetime is invalid")
            .kind(),
        LedgerErrorKind::InvalidConfiguration
    );

    let _ = UnixMillis::new(0);
}
