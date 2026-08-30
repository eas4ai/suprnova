//! Closed lazy-completion scheduling and lifecycle tests.

mod child_parameter_support;
mod component_support;

use std::collections::BTreeMap;

use component_support::{FixtureControl, install, metadata, trusted_context};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::child::verify_child_parameters;
use suprnova_live::component::composition::ChildParameterSchema;
use suprnova_live::component::lazy::{
    LazyCompletion, LazyExecutionMode, LazyMount, LazyPolicy, LazyPresentation,
    LazyPresentationState, LazyServerCompletion,
};
use suprnova_live::component::{ComponentExecutor, HydrationContext, RenderContext};
use suprnova_live::identity::{InstanceId, Revision, UnixMillis};
use suprnova_live::registry::ComponentDescriptor;

fn descriptor(control: std::sync::Arc<FixtureControl>) -> ComponentDescriptor {
    ComponentDescriptor::with_hooks(metadata().clone(), install(control)).with_composition(
        ChildParameterSchema::empty(1).expect("empty parameter schema"),
        true,
        true,
    )
}

#[test]
fn deferred_lazy_mounts_expose_semantic_ssr_state_and_only_typed_completion_intent() {
    let lazy = LazyMount::new(
        LazyPolicy::Deferred,
        LazyPresentation::new("Search results are loading").expect("semantic placeholder"),
    );

    let completion = lazy.schedule(LazyExecutionMode::Browser);
    let LazyCompletion::Deferred(request) = completion else {
        panic!("browser mode defers completion");
    };
    assert_eq!(request.presentation().text(), "Search results are loading");
    assert_eq!(request.operation().as_str(), "lazy_complete");
    assert_eq!(request.initial_state(), LazyPresentationState::Placeholder);
    assert_eq!(request.loading_state(), LazyPresentationState::Loading);
    assert!(!format!("{request:?}").contains("Search results"));
    assert_eq!(
        LazyServerCompletion::Empty.presentation_state(),
        LazyPresentationState::Empty
    );
    assert_eq!(
        LazyServerCompletion::Failed.presentation_state(),
        LazyPresentationState::Error
    );
    assert_eq!(
        LazyServerCompletion::Render.presentation_state(),
        LazyPresentationState::Success
    );

    assert!(matches!(
        lazy.schedule(LazyExecutionMode::TestEager),
        LazyCompletion::Eager
    ));
    assert!(matches!(
        lazy.schedule(LazyExecutionMode::NonBrowserEager),
        LazyCompletion::Eager
    ));
    assert!(LazyPresentation::new("").is_err());
    assert!(LazyPresentation::new(&"x".repeat(1_025)).is_err());
}

#[tokio::test]
async fn params_changed_and_lazy_complete_are_registered_child_local_lifecycle_operations() {
    let control = FixtureControl::new(component_support::FailurePoint::None);
    let descriptor = descriptor(control.clone());
    let request = trusted_context();
    let instance = InstanceId::from_bytes(&[0x61; 16]).expect("instance identity");
    let render = RenderContext::new(
        &request,
        &instance,
        Revision::new(4),
        UnixMillis::new(2_000),
    );
    let state = CanonicalValue::Object(BTreeMap::from([(
        "serial".to_owned(),
        CanonicalValue::String("1".to_owned()),
    )]));
    let hydration = HydrationContext::new(render, &state);
    let child_parameters = child_parameter_support::issued_child("verified").await;
    let verified = verify_child_parameters(
        &child_parameters.encoded,
        &child_parameters.expected,
        &child_parameters.keys,
        child_parameter_support::NOW,
        &child_parameters.limits,
    )
    .expect("child parameter authority verifies");

    ComponentExecutor::new()
        .params_changed(&descriptor, &hydration, &verified)
        .await
        .expect("registered parameter update lifecycle");
    assert!(control.values().contains(&"params_changed"));

    ComponentExecutor::new()
        .lazy_complete(&descriptor, &hydration)
        .await
        .expect("registered lazy lifecycle");
    assert!(control.values().contains(&"lazy_complete"));

    let unregistered = ComponentDescriptor::with_hooks(metadata().clone(), install(control));
    let error = ComponentExecutor::new()
        .lazy_complete(&unregistered, &hydration)
        .await
        .expect_err("unregistered operation cannot dispatch");
    assert_eq!(
        error.kind(),
        suprnova_live::component::LifecycleErrorKind::HooksUnavailable
    );
}
