//! Deterministic component reconstruction and lifecycle ordering.

mod component_support;

use std::collections::BTreeMap;

use component_support::{FailurePoint, FixtureControl, install, metadata, trusted_context};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{ComponentExecutor, HydrationContext, MountContext, RenderContext};
use suprnova_live::identity::{InstanceId, Revision, UnixMillis};
use suprnova_live::registry::ComponentDescriptor;

fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

fn contexts() -> (MountContext<'static>, RenderContext<'static>) {
    let instance = Box::leak(Box::new(
        InstanceId::from_bytes(&bytes::<16>(0x20)).expect("instance identity"),
    ));
    let parameters = Box::leak(Box::new(CanonicalValue::Object(BTreeMap::new())));
    let request = Box::leak(Box::new(trusted_context()));
    let render = RenderContext::new(request, instance, Revision::new(0), UnixMillis::new(5_000));
    let mount = MountContext::new(render, parameters);
    (mount, render)
}

#[tokio::test]
async fn initial_mount_has_one_mutation_aware_order_and_tears_down_once() {
    let control = FixtureControl::new(FailurePoint::None);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let output = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect("initial lifecycle succeeds");

    assert_eq!(
        output.state(),
        &CanonicalValue::Object(BTreeMap::from([(
            "serial".to_owned(),
            CanonicalValue::String("1".to_owned()),
        )]))
    );
    assert_eq!(output.render().body, "<p>1</p>");
    assert_eq!(
        control.values(),
        [
            "mount",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "memo",
            "teardown",
        ]
    );
}

#[tokio::test]
async fn action_reconstruction_uses_fresh_owned_instances_for_every_request() {
    let control = FixtureControl::new(FailurePoint::None);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (_, render) = contexts();
    let state = CanonicalValue::Null;
    let hydration = HydrationContext::new(render, &state);
    let executor = ComponentExecutor::new();

    let first = executor
        .reconstruct(&descriptor, &hydration)
        .await
        .expect("first reconstruction");
    let second = executor
        .reconstruct(&descriptor, &hydration)
        .await
        .expect("second reconstruction");

    assert_eq!(first.render().body, "<p>1</p>");
    assert_eq!(second.render().body, "<p>2</p>");
    assert_eq!(
        control.values(),
        [
            "reconstruct",
            "hydrated",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "memo",
            "teardown",
            "reconstruct",
            "hydrated",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "memo",
            "teardown",
        ]
    );
}
