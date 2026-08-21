//! Public-seed verification, promotion, replay, and scope tests.

mod promotion_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use promotion_support::{
    context, harness, nonce, promotion_limits, signed_seed, signed_seed_with_refresh,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::{BrowserNonce, Revision, UnixMillis};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::promotion::{PromotionErrorKind, PromotionService, RefreshBeforeAction};
use suprnova_live::snapshot::{ExpectedInstanceV1, verify_instance};

#[tokio::test]
async fn verified_seed_promotes_to_a_server_identified_scoped_instance() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let context = context(0x90);
    let browser_nonce = nonce(0x10);

    let promoted = harness
        .service
        .promote(&seed, browser_nonce.clone(), &context)
        .await
        .expect("valid seed promotes");

    assert_ne!(promoted.instance_id().as_bytes(), browser_nonce.as_bytes());
    assert_eq!(promoted.revision(), Revision::new(0));
    assert_eq!(
        promoted.refresh_before_action(),
        RefreshBeforeAction::Required
    );
    assert_eq!(promoted.advisory_generations().len(), 1);
    let expected = ExpectedInstanceV1::new(
        promotion_support::snapshot_support::component_contract(),
        suprnova_live::identity::BuildId::parse("build-2026-08-21").expect("build is valid"),
        promotion_support::snapshot_support::route(1),
        suprnova_live::identity::IslandSlot::parse("search-results").expect("slot is valid"),
        context.scope().clone(),
        promotion_support::snapshot_support::schema_set(),
    );
    let verified = verify_instance(
        promoted.signed_snapshot(),
        &expected,
        &harness.keys,
        UnixMillis::new(1_000),
        &harness.snapshot_limits,
    )
    .expect("promoted instance snapshot verifies");
    assert_eq!(verified.body().instance_id(), promoted.instance_id());
}

#[tokio::test]
async fn integrity_and_trusted_bindings_are_checked_before_identity_or_ledger_creation() {
    let harness = harness(promotion_limits(), 64);
    let mut tampered = signed_seed(&harness.keys, "rust");
    let position = tampered
        .iter()
        .position(|byte| *byte == b'r')
        .expect("seed contains test state");
    tampered[position] = b'x';

    let error = harness
        .service
        .promote(&tampered, nonce(0x11), &context(0x91))
        .await
        .expect_err("tampered seed fails closed");
    assert_eq!(error.kind(), PromotionErrorKind::SnapshotRejected);
    assert_eq!(harness.generator.calls(), 0);
    assert!(
        harness
            .ledger
            .inspect(
                &promotion_support::scope(0x91),
                &promotion_support::instance(0xd0),
            )
            .expect("inspection succeeds")
            .is_none()
    );
}

#[tokio::test]
async fn valid_signature_with_the_wrong_current_route_fails_before_identity_generation() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let wrong_context = suprnova_live::promotion::TrustedPromotionContext::new(
        suprnova_live::snapshot::ExpectedSeedV1::new(
            promotion_support::snapshot_support::component_contract(),
            suprnova_live::identity::BuildId::parse("build-2026-08-21").expect("build is valid"),
            promotion_support::snapshot_support::route(2),
            suprnova_live::identity::IslandSlot::parse("search-results").expect("slot is valid"),
            promotion_support::snapshot_support::schema_set(),
        ),
        promotion_support::scope(0x95),
        suprnova_live::promotion::PromotionAttestations::verified(),
    );

    assert_eq!(
        harness
            .service
            .promote(&seed, nonce(0x15), &wrong_context)
            .await
            .expect_err("current route binding must match")
            .kind(),
        PromotionErrorKind::SnapshotRejected
    );
    assert_eq!(harness.generator.calls(), 0);
}

#[tokio::test]
async fn exact_retry_recovers_one_instance_while_new_nonce_and_scope_are_independent() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let first_context = context(0x92);
    let first_nonce = nonce(0x12);
    let first = harness
        .service
        .promote(&seed, first_nonce.clone(), &first_context)
        .await
        .expect("first promotion succeeds");
    let retry = harness
        .service
        .promote(&seed, first_nonce.clone(), &first_context)
        .await
        .expect("exact retry recovers");
    assert_eq!(retry.instance_id(), first.instance_id());

    let independent = harness
        .service
        .promote(&seed, nonce(0x13), &first_context)
        .await
        .expect("new nonce creates independent instance");
    assert_ne!(independent.instance_id(), first.instance_id());

    let other_scope = harness
        .service
        .promote(&seed, first_nonce, &context(0x93))
        .await
        .expect("same public seed and nonce in another scope stays independent");
    assert_ne!(other_scope.instance_id(), first.instance_id());
}

#[tokio::test]
async fn nonce_reuse_with_changed_signed_input_is_rejected() {
    let harness = harness(promotion_limits(), 64);
    let context = context(0x94);
    let nonce = nonce(0x14);
    harness
        .service
        .promote(&signed_seed(&harness.keys, "rust"), nonce.clone(), &context)
        .await
        .expect("first promotion succeeds");

    let error = harness
        .service
        .promote(&signed_seed(&harness.keys, "other"), nonce, &context)
        .await
        .expect_err("same nonce cannot identify changed signed input");
    assert_eq!(error.kind(), PromotionErrorKind::NonceConflict);
}

#[test]
fn browser_nonce_type_rejects_less_than_128_bits() {
    assert!(BrowserNonce::from_bytes(&[0_u8; 15]).is_err());
}

#[tokio::test]
async fn refresh_on_promote_is_a_typed_component_choice_not_a_coherence_gate() {
    let harness = harness(promotion_limits(), 64);
    let promoted = harness
        .service
        .promote(
            &signed_seed_with_refresh(&harness.keys, "rust", false),
            nonce(0x16),
            &context(0x96),
        )
        .await
        .expect("advisory generations do not reject promotion");
    assert_eq!(
        promoted.refresh_before_action(),
        RefreshBeforeAction::NotRequired
    );
    assert_eq!(promoted.advisory_generations().len(), 1);
}

#[derive(Debug)]
struct CompletionClock {
    calls: AtomicUsize,
}

impl Clock for CompletionClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(UnixMillis::new(if call < 2 { 1_000 } else { 1_101 }))
    }
}

#[tokio::test]
async fn promotion_completion_after_the_policy_lease_fails_closed() {
    let clock = Arc::new(CompletionClock {
        calls: AtomicUsize::new(0),
    });
    let ledger = Arc::new(MemoryInstanceLedger::new(
        clock.clone(),
        LedgerLimits::new(100, 10_000, 4, 64).expect("ledger limits are valid"),
    ));
    let generator = Arc::new(promotion_support::SequenceGenerator::new(0xd0));
    let keys = Arc::new(promotion_support::snapshot_support::key_ring());
    let snapshot_limits = promotion_support::snapshot_support::snapshot_limits();
    let service = PromotionService::new(
        ledger,
        clock,
        generator,
        keys.clone(),
        snapshot_limits,
        promotion_limits(),
    )
    .expect("promotion service config is valid");

    let error = service
        .promote(&signed_seed(&keys, "rust"), nonce(0x17), &context(0x97))
        .await
        .expect_err("completion after the promotion lease must fail closed");
    assert_eq!(error.kind(), PromotionErrorKind::ProviderInvariant);
}

#[test]
fn production_instance_generator_returns_128_bits_of_server_identity() {
    use suprnova_live::promotion::{InstanceIdGenerator, SystemInstanceIdGenerator};

    let instance = SystemInstanceIdGenerator
        .generate()
        .expect("operating-system randomness is available");
    assert_eq!(instance.as_bytes().len(), 16);
}
