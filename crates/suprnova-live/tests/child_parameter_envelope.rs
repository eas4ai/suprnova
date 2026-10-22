//! Signed child-parameter capability and accepted-parent publication tests.

mod child_parameter_support;

use child_parameter_support::{
    EXPIRES, NOW, accepted_parent, key_ring, parameter_limits, pending_parameters,
};
use suprnova_live::child::{
    ExpectedChildParametersV1, PreparedChildParametersV1, verify_child_parameters,
};

#[tokio::test]
async fn accepted_parent_outcome_publishes_one_typed_child_parameter_capability() {
    let parent = accepted_parent().await;
    let (pending, schema) = pending_parameters("zig");
    let keys = key_ring("child-v1", 0x33);
    let limits = parameter_limits();
    let prepared = PreparedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending.clone(),
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("parent render prepares a bounded update");

    let encoded = prepared
        .publish(&parent.accepted, &keys, NOW, &limits)
        .expect("accepted parent revision may publish");
    let expected = ExpectedChildParametersV1::new(
        parent.scope,
        parent.instance,
        parent.revision,
        pending.child().key().clone(),
        pending.child().component_contract().clone(),
        schema,
    );
    let verified = verify_child_parameters(&encoded, &expected, &keys, NOW, &limits)
        .expect("matching envelope verifies");

    assert_eq!(verified.parameters(), pending.parameters());
    assert_eq!(verified.parent_revision(), parent.revision);
    assert_eq!(verified.child_key(), pending.child().key());
    assert!(!format!("{verified:?}").contains("zig"));

    let replay = expected.after_applied_parent_revision(parent.revision);
    let error = verify_child_parameters(&encoded, &replay, &keys, NOW, &limits)
        .expect_err("a child records and rejects an already applied parent revision");
    assert_eq!(
        error.kind(),
        suprnova_live::child::ChildParameterErrorKind::ParentRevisionMismatch
    );
}

#[tokio::test]
async fn a_rendered_draft_is_not_publishable_for_a_different_accepted_parent() {
    let parent = accepted_parent().await;
    let (pending, _) = pending_parameters("zig");
    let keys = key_ring("child-v1", 0x33);
    let limits = parameter_limits();
    let prepared = PreparedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision.checked_next().expect("successor"),
        pending,
        NOW,
        EXPIRES,
        keys.active_key_id().clone(),
        &limits,
    )
    .expect("future render may prepare before acceptance");

    let error = prepared
        .publish(&parent.accepted, &keys, NOW, &limits)
        .expect_err("mismatched acceptance cannot publish");
    assert_eq!(
        error.kind(),
        suprnova_live::child::ChildParameterErrorKind::ParentNotAccepted
    );
}
