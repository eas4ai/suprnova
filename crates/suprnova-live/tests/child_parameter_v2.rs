//! Exact-child-bound child-parameter envelope v2 and server eligibility tests.

mod child_parameter_support;
mod snapshot_support;

use std::error::Error;
use std::sync::Arc;

use child_parameter_support::{
    EXPIRES, NOW, accepted_parent, instance, issued_child, key_ring, parameter_limits,
    parameter_schema, pending_parameters,
};
use suprnova_live::child::{
    ChildParameterEligibilityErrorKind, ChildParameterErrorKind, ExpectedChildParametersV2,
    PreparedChildParametersV2, VerifiedChildParametersV2, authorize_child_parameters_v2,
    verify_child_parameters_v2,
};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::component::composition::ChildKey;
use suprnova_live::identity::{
    BuildId, ContentDigest, IdempotencyKey, InstanceId, IslandSlot, Revision, ScopeFingerprint,
    UnixMillis,
};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LedgerError, LedgerErrorKind,
    LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::snapshot::{
    CompositionChildLineageV1, CompositionLineageV1, ExpectedInstanceV1, InstanceBodyV1,
    SnapshotLimits, VerifiedInstanceV1, verify_instance,
};

struct EligibleFixture {
    parameters: VerifiedChildParametersV2,
    parent_snapshot: VerifiedInstanceV1,
    ledger: MemoryInstanceLedger,
    parent_scope: ScopeFingerprint,
    parent_instance: InstanceId,
    parent_revision: Revision,
}

#[derive(Debug)]
struct FailingClock;

impl Clock for FailingClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Err(ClockError::timestamp_overflow())
    }
}

fn verified_parent_snapshot(
    parent_scope: ScopeFingerprint,
    parent_instance: InstanceId,
    parent_revision: Revision,
    child_key: ChildKey,
    child_contract: ContentDigest,
    child_instance: InstanceId,
) -> VerifiedInstanceV1 {
    let snapshot_keys = snapshot_support::key_ring();
    let snapshot_limits = SnapshotLimits::new(
        InputLimits::new(16 * 1024, 8, 256, 512).expect("snapshot input limits"),
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .expect("snapshot limits");
    let schemas = snapshot_support::schema_set();
    let mut fields = snapshot_support::instance_fields(&snapshot_keys);
    fields.scope = parent_scope;
    fields.instance_id = parent_instance;
    fields.revision = parent_revision;
    fields
        .set_composition_lineage(
            CompositionLineageV1::new(
                None,
                vec![
                    CompositionChildLineageV1::new(
                        fields.instance_id.clone(),
                        fields.revision,
                        child_key,
                        child_contract,
                        child_instance,
                        1,
                    )
                    .expect("child lineage"),
                ],
            )
            .expect("composition lineage"),
        )
        .expect("lineage installs");
    let expected_scope = fields.scope.clone();
    let parent_snapshot = InstanceBodyV1::new(fields, &schemas, &snapshot_limits)
        .expect("parent snapshot")
        .sign(&snapshot_keys, UnixMillis::new(1_010), &snapshot_limits)
        .expect("parent snapshot signs");
    let expected_snapshot = ExpectedInstanceV1::new(
        snapshot_support::component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id"),
        snapshot_support::route(1),
        IslandSlot::parse("search-results").expect("slot"),
        expected_scope,
        schemas,
    );
    verify_instance(
        &parent_snapshot,
        &expected_snapshot,
        &snapshot_keys,
        UnixMillis::new(1_050),
        &snapshot_limits,
    )
    .expect("parent snapshot verifies")
}

async fn eligible_fixture() -> EligibleFixture {
    let parent = accepted_parent().await;
    let (pending, schema) = pending_parameters("eligible");
    let child_instance = instance(0x92);
    let keys = key_ring("child-v2-eligible", 0x45);
    let limits = parameter_limits();
    let prepared = PreparedChildParametersV2::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        child_instance.clone(),
        pending.clone(),
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("v2 child parameters prepare");
    let encoded = prepared
        .publish(&parent.accepted, &keys, NOW, &limits)
        .expect("accepted parent publishes v2");
    let expected = ExpectedChildParametersV2::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending.child().key().clone(),
        pending.child().component_contract().clone(),
        child_instance.clone(),
        schema,
    );
    let parameters = verify_child_parameters_v2(&encoded, &expected, &keys, NOW, &limits)
        .expect("v2 envelope verifies");

    let parent_snapshot = verified_parent_snapshot(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending.child().key().clone(),
        pending.child().component_contract().clone(),
        child_instance,
    );

    EligibleFixture {
        parameters,
        parent_snapshot,
        ledger: parent.ledger,
        parent_scope: parent.scope,
        parent_instance: parent.instance,
        parent_revision: parent.revision,
    }
}

#[tokio::test]
async fn v2_envelope_binds_the_exact_child_instance_without_reinterpreting_v1() {
    let parent = accepted_parent().await;
    let (pending, schema) = pending_parameters("zig");
    let child_instance = instance(0x91);
    let keys = key_ring("child-v2", 0x44);
    let limits = parameter_limits();
    let prepared = PreparedChildParametersV2::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        child_instance.clone(),
        pending.clone(),
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("v2 child parameters prepare");
    let encoded = prepared
        .publish(&parent.accepted, &keys, NOW, &limits)
        .expect("accepted parent publishes v2");
    let expected = ExpectedChildParametersV2::new(
        parent.scope,
        parent.instance,
        parent.revision,
        pending.child().key().clone(),
        pending.child().component_contract().clone(),
        child_instance.clone(),
        schema,
    );

    let verified = verify_child_parameters_v2(&encoded, &expected, &keys, NOW, &limits)
        .expect("matching exact child verifies as v2");

    assert_eq!(verified.child_instance(), &child_instance);
    assert_eq!(verified.parameters(), pending.parameters());
}

#[tokio::test]
async fn v2_verification_rejects_v1_and_every_foreign_exact_binding() {
    let v1 = issued_child("historical-v1").await;
    let v1_as_v2 = ExpectedChildParametersV2::new(
        v1.parent_scope,
        v1.parent_instance,
        v1.parent_revision,
        v1.child_key,
        v1.child_contract,
        instance(0x93),
        parameter_schema(),
    );
    assert_eq!(
        verify_child_parameters_v2(&v1.encoded, &v1_as_v2, &v1.keys, NOW, &v1.limits)
            .expect_err("v1 is never reinterpreted as exact-child v2")
            .kind(),
        ChildParameterErrorKind::UnsupportedSchema
    );

    let parent = accepted_parent().await;
    let (pending, schema) = pending_parameters("binding-matrix");
    let child_instance = instance(0x94);
    let keys = key_ring("child-v2-bindings", 0x46);
    let limits = parameter_limits();
    let encoded = PreparedChildParametersV2::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        child_instance.clone(),
        pending.clone(),
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("v2 prepares")
    .publish(&parent.accepted, &keys, NOW, &limits)
    .expect("v2 publishes");
    let cases = [
        ExpectedChildParametersV2::new(
            child_parameter_support::scope(0xd0),
            parent.instance.clone(),
            parent.revision,
            pending.child().key().clone(),
            pending.child().component_contract().clone(),
            child_instance.clone(),
            schema.clone(),
        ),
        ExpectedChildParametersV2::new(
            parent.scope.clone(),
            instance(0xd1),
            parent.revision,
            pending.child().key().clone(),
            pending.child().component_contract().clone(),
            child_instance.clone(),
            schema.clone(),
        ),
        ExpectedChildParametersV2::new(
            parent.scope.clone(),
            parent.instance.clone(),
            parent.revision,
            ChildKey::parse("foreign-key").expect("foreign key"),
            pending.child().component_contract().clone(),
            child_instance.clone(),
            schema.clone(),
        ),
        ExpectedChildParametersV2::new(
            parent.scope.clone(),
            parent.instance.clone(),
            parent.revision,
            pending.child().key().clone(),
            child_parameter_support::digest(0xd2),
            child_instance.clone(),
            schema.clone(),
        ),
        ExpectedChildParametersV2::new(
            parent.scope,
            parent.instance,
            parent.revision,
            pending.child().key().clone(),
            pending.child().component_contract().clone(),
            instance(0xd3),
            schema,
        ),
    ];

    for expected in cases {
        assert_eq!(
            verify_child_parameters_v2(&encoded, &expected, &keys, NOW, &limits)
                .expect_err("foreign scope, parent, key, component, or child fails closed")
                .kind(),
            ChildParameterErrorKind::BindingMismatch
        );
    }
}

#[tokio::test]
async fn server_eligibility_requires_signed_lineage_and_current_ledger_authority() {
    let fixture = eligible_fixture().await;
    let eligible = authorize_child_parameters_v2(
        &fixture.parameters,
        &fixture.parent_snapshot,
        &fixture.ledger,
    )
    .await
    .expect("matching signed lineage and ledger authority authorize the child");

    assert_eq!(
        eligible.child_instance(),
        fixture.parameters.child_instance()
    );
    assert_eq!(eligible.parameters(), fixture.parameters.parameters());
}

#[tokio::test]
async fn accepted_later_parent_revision_invalidates_the_same_signed_v2_envelope() {
    let fixture = eligible_fixture().await;
    let request = ClaimRequest::new(
        fixture.parent_scope.clone(),
        fixture.parent_instance.clone(),
        fixture.parent_revision,
        IdempotencyKey::from_bytes(&child_parameter_support::bytes::<16>(0xa0))
            .expect("idempotency key"),
        child_parameter_support::digest(0xb0),
    );
    let grant = match fixture.ledger.claim(request).await.expect("next claim") {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected next claim, got {other:?}"),
    };
    fixture
        .ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(
                AcceptedOutcomeKind::Rendered,
                child_parameter_support::digest(0xc0),
            ),
        )
        .await
        .expect("later parent revision commits");

    let error = authorize_child_parameters_v2(
        &fixture.parameters,
        &fixture.parent_snapshot,
        &fixture.ledger,
    )
    .await
    .expect_err("a browser snapshot cannot preserve superseded parent authority");

    assert_eq!(
        error.kind(),
        ChildParameterEligibilityErrorKind::ParentRevisionMismatch
    );
}

#[tokio::test]
async fn eligibility_rejects_foreign_parent_or_signed_child_lineage() {
    let fixture = eligible_fixture().await;
    let (pending, _) = pending_parameters("eligible");
    let correct_key = pending.child().key().clone();
    let correct_contract = pending.child().component_contract().clone();
    let correct_child = instance(0x92);
    let cases = [
        (
            verified_parent_snapshot(
                child_parameter_support::scope(0xd4),
                fixture.parent_instance.clone(),
                fixture.parent_revision,
                correct_key.clone(),
                correct_contract.clone(),
                correct_child.clone(),
            ),
            ChildParameterEligibilityErrorKind::BindingMismatch,
        ),
        (
            verified_parent_snapshot(
                fixture.parent_scope.clone(),
                instance(0xd5),
                fixture.parent_revision,
                correct_key.clone(),
                correct_contract.clone(),
                correct_child.clone(),
            ),
            ChildParameterEligibilityErrorKind::BindingMismatch,
        ),
        (
            verified_parent_snapshot(
                fixture.parent_scope.clone(),
                fixture.parent_instance.clone(),
                fixture.parent_revision,
                ChildKey::parse("foreign-lineage-key").expect("foreign key"),
                correct_contract.clone(),
                correct_child.clone(),
            ),
            ChildParameterEligibilityErrorKind::CompositionLineageMismatch,
        ),
        (
            verified_parent_snapshot(
                fixture.parent_scope.clone(),
                fixture.parent_instance.clone(),
                fixture.parent_revision,
                correct_key.clone(),
                child_parameter_support::digest(0xd6),
                correct_child,
            ),
            ChildParameterEligibilityErrorKind::CompositionLineageMismatch,
        ),
        (
            verified_parent_snapshot(
                fixture.parent_scope.clone(),
                fixture.parent_instance.clone(),
                fixture.parent_revision,
                correct_key,
                correct_contract,
                instance(0xd7),
            ),
            ChildParameterEligibilityErrorKind::CompositionLineageMismatch,
        ),
    ];

    for (parent_snapshot, expected_kind) in cases {
        let error =
            authorize_child_parameters_v2(&fixture.parameters, &parent_snapshot, &fixture.ledger)
                .await
                .expect_err("foreign parent or signed child lineage fails closed");
        assert_eq!(error.kind(), expected_kind);
    }
}

#[tokio::test]
async fn missing_or_failing_ledger_authority_fails_closed_with_provider_cause() {
    let fixture = eligible_fixture().await;
    let ledger_limits = LedgerLimits::new(100, 10_000, 8, 64).expect("ledger limits");
    let empty = MemoryInstanceLedger::new(
        Arc::new(suprnova_live::clock::SystemClock) as Arc<dyn Clock>,
        ledger_limits,
    );
    let missing =
        authorize_child_parameters_v2(&fixture.parameters, &fixture.parent_snapshot, &empty)
            .await
            .expect_err("a valid browser snapshot cannot replace missing ledger authority");
    assert_eq!(
        missing.kind(),
        ChildParameterEligibilityErrorKind::ParentAuthorityMissing
    );

    let failing =
        MemoryInstanceLedger::new(Arc::new(FailingClock) as Arc<dyn Clock>, ledger_limits);
    let provider =
        authorize_child_parameters_v2(&fixture.parameters, &fixture.parent_snapshot, &failing)
            .await
            .expect_err("provider error cannot authorize the child");
    assert_eq!(
        provider.kind(),
        ChildParameterEligibilityErrorKind::ProviderUnavailable
    );
    assert_eq!(
        provider
            .source()
            .and_then(|source| source.downcast_ref::<LedgerError>())
            .map(|error| error.kind()),
        Some(LedgerErrorKind::ClockUnavailable)
    );
}
