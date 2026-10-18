//! Typed child composition, identity, transition, and bound tests.

use std::collections::BTreeMap;

use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::composition::{
    ChildDeclaration, ChildKey, ChildParameterField, ChildParameterSchema, ChildState,
    CompositionAncestry, CompositionErrorKind, CompositionLimits, CompositionPlanner,
};
use suprnova_live::identity::{ComponentName, InstanceId, ModelField, ViewName};
use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistry, ComponentRegistryBuilder};
use suprnova_live::state::ModelCodec;

fn parameters(query: CanonicalValue) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([("query".to_owned(), query)]))
}

fn schema() -> ChildParameterSchema {
    ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(
            ModelField::parse("query").expect("parameter name"),
            ModelCodec::String,
            true,
        )],
    )
    .expect("parameter schema")
}

fn descriptor(name: &str, component_version: u16) -> ComponentDescriptor {
    descriptor_with_capabilities(name, component_version, true, true)
}

fn descriptor_with_capabilities(
    name: &str,
    component_version: u16,
    params_changed: bool,
    lazy_complete: bool,
) -> ComponentDescriptor {
    let metadata = ComponentMetadata::new(
        ComponentName::parse(name).expect("component name"),
        ViewName::parse(&format!("live/{}.html", name.replace('.', "/"))).expect("view name"),
        ContractVersions::new(component_version, 1, 1, 1, 1).expect("versions"),
        vec![],
        vec![],
    )
    .expect("metadata");
    ComponentDescriptor::new(metadata).with_composition(schema(), params_changed, lazy_complete)
}

fn registry(component_version: u16) -> ComponentRegistry {
    ComponentRegistryBuilder::new()
        .register(descriptor("catalog.results", component_version))
        .expect("component registers")
        .build()
}

fn declaration(key: &str, query: &str) -> ChildDeclaration {
    ChildDeclaration::new(
        ChildKey::parse(key).expect("child key"),
        ComponentName::parse("catalog.results").expect("component name"),
        parameters(CanonicalValue::String(query.to_owned())),
    )
}

fn planner(max_children: usize, max_pending: usize) -> CompositionPlanner {
    CompositionPlanner::new(
        CompositionLimits::new(max_children, 8, 4_096, max_pending).expect("composition limits"),
    )
}

#[test]
fn typed_parameters_and_stable_keys_fail_before_a_child_transition_exists() {
    let current_registry = registry(1);
    let ancestry = CompositionAncestry::root(
        ComponentName::parse("catalog.search").expect("parent component"),
    );
    let planner = planner(4, 4);

    let error = planner
        .reconcile(
            &current_registry,
            &ancestry,
            &[],
            vec![ChildDeclaration::new(
                ChildKey::parse("results").expect("child key"),
                ComponentName::parse("catalog.results").expect("component"),
                parameters(CanonicalValue::Bool(true)),
            )],
        )
        .expect_err("wrong parameter type is rejected");
    assert_eq!(error.kind(), CompositionErrorKind::InvalidParameters);

    assert!(ChildKey::parse("").is_err());
    assert!(ChildKey::parse("unstable key").is_err());
    assert!(ChildKey::parse(&"x".repeat(129)).is_err());
}

#[test]
fn child_reconciliation_preserves_ownership_and_classifies_every_survivor_state() {
    let current_registry = registry(1);
    let ancestry = CompositionAncestry::root(
        ComponentName::parse("catalog.search").expect("parent component"),
    );
    let planner = planner(4, 4);

    let initial = planner
        .reconcile(
            &current_registry,
            &ancestry,
            &[],
            vec![declaration("results", "rust")],
        )
        .expect("initial child plan");
    let [ChildState::Remount(prepared)] = initial.as_slice() else {
        panic!("new child remounts");
    };
    let instance = InstanceId::from_bytes(&[0x44; 16]).expect("child instance");
    let handle = prepared.clone().into_handle(instance.clone());

    let unchanged = planner
        .reconcile(
            &current_registry,
            &ancestry,
            std::slice::from_ref(&handle),
            vec![declaration("results", "rust")],
        )
        .expect("unchanged child plan");
    let [ChildState::Unchanged(survivor)] = unchanged.as_slice() else {
        panic!("identical child survives");
    };
    assert_eq!(survivor.instance_id(), &instance);

    let changed = planner
        .reconcile(
            &current_registry,
            &ancestry,
            std::slice::from_ref(&handle),
            vec![declaration("results", "zig")],
        )
        .expect("changed child plan");
    let [ChildState::PendingParams(pending)] = changed.as_slice() else {
        panic!("parameter change becomes pending");
    };
    assert_eq!(pending.child().instance_id(), &instance);
    assert_eq!(
        pending.parameters(),
        &parameters(CanonicalValue::String("zig".to_owned()))
    );

    let removed = planner
        .reconcile(
            &current_registry,
            &ancestry,
            std::slice::from_ref(&handle),
            vec![],
        )
        .expect("removed child plan");
    assert!(matches!(removed.as_slice(), [ChildState::Removed(_)]));

    let drifted_registry = registry(2);
    let drifted = planner
        .reconcile(
            &drifted_registry,
            &ancestry,
            &[handle],
            vec![declaration("results", "rust")],
        )
        .expect("contract drift plan");
    assert!(matches!(drifted.as_slice(), [ChildState::Remount(_)]));

    let capability_registry = ComponentRegistryBuilder::new()
        .register(descriptor_with_capabilities(
            "catalog.results",
            1,
            false,
            true,
        ))
        .expect("component registers")
        .build();
    let capability_drift = planner
        .reconcile(
            &capability_registry,
            &ancestry,
            &[prepared
                .clone()
                .into_handle(InstanceId::from_bytes(&[0x45; 16]).expect("child instance"))],
            vec![declaration("results", "rust")],
        )
        .expect("lifecycle capability drift plan");
    assert!(matches!(
        capability_drift.as_slice(),
        [ChildState::Remount(_)]
    ));
}

#[test]
fn duplicate_capacity_pending_and_circular_composition_are_hard_bounded() {
    let current_registry = registry(1);
    let parent = ComponentName::parse("catalog.search").expect("parent component");
    let ancestry = CompositionAncestry::root(parent);
    let duplicate_planner = planner(2, 1);

    let duplicate = duplicate_planner
        .reconcile(
            &current_registry,
            &ancestry,
            &[],
            vec![declaration("same", "a"), declaration("same", "b")],
        )
        .expect_err("duplicate keys fail");
    assert_eq!(duplicate.kind(), CompositionErrorKind::DuplicateKey);

    let capacity = planner(1, 1)
        .reconcile(
            &current_registry,
            &ancestry,
            &[],
            vec![declaration("one", "a"), declaration("two", "b")],
        )
        .expect_err("child count fails");
    assert_eq!(capacity.kind(), CompositionErrorKind::TooManyChildren);

    let recursive = ancestry
        .enter(
            ChildKey::parse("self").expect("child key"),
            ComponentName::parse("catalog.search").expect("same component"),
            8,
        )
        .expect_err("component ancestry cycle fails");
    assert_eq!(recursive.kind(), CompositionErrorKind::CircularComposition);

    let shallow =
        CompositionAncestry::root(ComponentName::parse("catalog.root").expect("root component"));
    let level_two = shallow
        .enter(
            ChildKey::parse("child").expect("child key"),
            ComponentName::parse("catalog.child").expect("child component"),
            2,
        )
        .expect("second level");
    let depth = level_two
        .enter(
            ChildKey::parse("grandchild").expect("child key"),
            ComponentName::parse("catalog.grandchild").expect("child component"),
            2,
        )
        .expect_err("depth is bounded");
    assert_eq!(depth.kind(), CompositionErrorKind::DepthExceeded);
}

#[test]
fn pending_parameter_capacity_and_child_failure_recovery_never_roll_back_the_parent() {
    let current_registry = registry(1);
    let ancestry = CompositionAncestry::root(
        ComponentName::parse("catalog.search").expect("parent component"),
    );
    let initial = planner(2, 2)
        .reconcile(
            &current_registry,
            &ancestry,
            &[],
            vec![declaration("one", "a"), declaration("two", "b")],
        )
        .expect("initial plan");
    let handles = initial
        .into_iter()
        .enumerate()
        .map(|(index, state)| match state {
            ChildState::Remount(prepared) => prepared.into_handle(
                InstanceId::from_bytes(&[0x50 + index as u8; 16]).expect("child instance"),
            ),
            _ => panic!("initial child remounts"),
        })
        .collect::<Vec<_>>();

    let error = planner(2, 1)
        .reconcile(
            &current_registry,
            &ancestry,
            &handles,
            vec![declaration("one", "c"), declaration("two", "d")],
        )
        .expect_err("pending transitions are bounded");
    assert_eq!(error.kind(), CompositionErrorKind::TooManyPending);

    let recovery =
        suprnova_live::component::composition::ChildFailureRecovery::for_child(handles[0].clone());
    assert_eq!(recovery.child().key().as_str(), "one");
    assert!(!recovery.rolls_back_parent());
}
