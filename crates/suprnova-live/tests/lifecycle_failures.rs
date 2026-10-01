//! Lifecycle short-circuit, panic, and cleanup behavior.

mod component_support;

use std::collections::BTreeMap;

use component_support::{FailurePoint, FixtureControl, install, metadata, trusted_context};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{
    ComponentExecutor, LifecycleErrorKind, LifecyclePhase, MountContext, RenderContext,
};
use suprnova_live::identity::{InstanceId, Revision, UnixMillis};
use suprnova_live::registry::ComponentDescriptor;

fn contexts() -> (MountContext<'static>, RenderContext<'static>) {
    let instance = Box::leak(Box::new(
        InstanceId::from_bytes(&[0x20; 16]).expect("instance identity"),
    ));
    let parameters = Box::leak(Box::new(CanonicalValue::Object(BTreeMap::new())));
    let request = Box::leak(Box::new(trusted_context()));
    let render = RenderContext::new(request, instance, Revision::new(0), UnixMillis::new(5_000));
    let mount = MountContext::new(render, parameters);
    (mount, render)
}

#[tokio::test]
async fn hook_failure_suppresses_downstream_phases_but_tears_down_once() {
    let control = FixtureControl::new(FailurePoint::Rendering);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("rendering hook fails");

    assert_eq!(error.kind(), LifecycleErrorKind::ComponentFailure);
    assert_eq!(error.phase(), LifecyclePhase::Rendering);
    assert_eq!(control.values(), ["mount", "rendering", "teardown"]);
}

#[tokio::test]
async fn panic_is_classified_and_never_skips_owned_teardown() {
    let control = FixtureControl::new(FailurePoint::RenderPanic);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("panic becomes a closed internal failure");

    assert_eq!(error.kind(), LifecycleErrorKind::Panicked);
    assert_eq!(error.phase(), LifecyclePhase::Render);
    assert_eq!(
        control.values(),
        ["mount", "rendering", "render", "teardown"]
    );
}

#[tokio::test]
async fn teardown_failure_is_reported_after_success_and_not_retried() {
    let control = FixtureControl::new(FailurePoint::Teardown);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("teardown failure prevents successful publication");

    assert_eq!(error.kind(), LifecycleErrorKind::ComponentFailure);
    assert_eq!(error.phase(), LifecyclePhase::Teardown);
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "teardown")
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_mount_has_no_instance_to_teardown() {
    let control = FixtureControl::new(FailurePoint::Mount);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("mount fails");

    assert_eq!(error.phase(), LifecyclePhase::Mount);
    assert_eq!(control.values(), ["mount"]);
}

#[tokio::test]
async fn dehydration_failure_suppresses_publication_and_still_tears_down_once() {
    let control = FixtureControl::new(FailurePoint::Dehydrate);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("dehydration fails");

    assert_eq!(error.kind(), LifecycleErrorKind::ComponentFailure);
    assert_eq!(error.phase(), LifecyclePhase::Dehydrate);
    assert_eq!(
        control.values(),
        [
            "mount",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "teardown",
        ]
    );
}

#[tokio::test]
async fn component_drop_panic_is_contained_after_exactly_one_teardown() {
    let control = FixtureControl::new(FailurePoint::DropPanic);
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
    let (mount, _render) = contexts();

    let error = ComponentExecutor::new()
        .initial_mount(&descriptor, &mount)
        .await
        .expect_err("drop panic becomes a closed teardown failure");

    assert_eq!(error.kind(), LifecycleErrorKind::Panicked);
    assert_eq!(error.phase(), LifecyclePhase::Teardown);
    assert_eq!(
        control
            .values()
            .into_iter()
            .filter(|phase| *phase == "teardown")
            .count(),
        1
    );
}

#[tokio::test]
async fn every_synchronous_and_future_drop_panic_stays_inside_the_lifecycle_boundary() {
    for (failure, phase, tears_down) in [
        (FailurePoint::MountCallPanic, LifecyclePhase::Mount, false),
        (FailurePoint::MetadataPanic, LifecyclePhase::Mount, true),
        (FailurePoint::RenderCallPanic, LifecyclePhase::Render, true),
        (
            FailurePoint::RenderFutureDropPanic,
            LifecyclePhase::Render,
            true,
        ),
        (
            FailurePoint::TeardownCallPanic,
            LifecyclePhase::Teardown,
            true,
        ),
    ] {
        let control = FixtureControl::new(failure);
        let descriptor =
            ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()));
        let (mount, _render) = contexts();
        let error = ComponentExecutor::new()
            .initial_mount(&descriptor, &mount)
            .await
            .expect_err("plugin panic is contained");

        assert_eq!(error.kind(), LifecycleErrorKind::Panicked, "{failure:?}");
        assert_eq!(error.phase(), phase, "{failure:?}");
        assert_eq!(
            control
                .values()
                .into_iter()
                .filter(|entry| *entry == "teardown")
                .count(),
            usize::from(tears_down),
            "{failure:?}"
        );
    }
}
