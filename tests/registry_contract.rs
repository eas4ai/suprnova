//! Explicit immutable component-registry contracts.

use suprnova_live::identity::{ActionName, ComponentName, ViewName};
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder, RegistryErrorKind};

fn descriptor(component: &str, view: &str, action: &str) -> ComponentDescriptor {
    let metadata = ComponentMetadata::new(
        ComponentName::parse(component).expect("component identity"),
        ViewName::parse(view).expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
        vec![],
        vec![
            ActionMetadata::new(ActionName::parse(action).expect("action identity"), 1)
                .expect("action metadata"),
        ],
    )
    .expect("metadata");
    ComponentDescriptor::new(metadata)
}

#[test]
fn explicit_registry_rejects_duplicate_component_and_view_ownership() {
    let duplicate_component = ComponentRegistryBuilder::new()
        .register(descriptor(
            "account.profile",
            "components/account/profile.html",
            "save",
        ))
        .expect("first registration")
        .register(descriptor(
            "account.profile",
            "components/account/other.html",
            "reset",
        ))
        .expect_err("duplicate component");
    assert_eq!(
        duplicate_component.kind(),
        RegistryErrorKind::DuplicateComponent
    );

    let duplicate_view = ComponentRegistryBuilder::new()
        .register(descriptor(
            "account.profile",
            "components/account/profile.html",
            "save",
        ))
        .expect("first registration")
        .register(descriptor(
            "account.security",
            "components/account/profile.html",
            "reset",
        ))
        .expect_err("duplicate view");
    assert_eq!(duplicate_view.kind(), RegistryErrorKind::DuplicateView);
}

#[test]
fn immutable_registry_resolves_only_explicit_component_contracts() {
    let descriptor = descriptor("account.profile", "components/account/profile.html", "save");
    let expected_digest = descriptor.contract_digest().clone();
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("registration")
        .build();

    let component = ComponentName::parse("account.profile").expect("component identity");
    assert_eq!(
        registry
            .resolve(&component)
            .expect("registered component")
            .contract_digest(),
        &expected_digest
    );
    assert!(
        registry
            .require_contract(&component, &expected_digest)
            .is_ok()
    );

    let missing = ComponentName::parse("browser.selected.type").expect("component identity");
    let error = registry
        .resolve(&missing)
        .expect_err("unregistered component");
    assert_eq!(error.kind(), RegistryErrorKind::NotRegistered);
}

#[test]
fn contract_mismatch_fails_without_exposing_browser_identity() {
    let registered = descriptor("account.profile", "components/account/profile.html", "save");
    let other_digest = descriptor(
        "account.security",
        "components/account/security.html",
        "reset",
    )
    .contract_digest()
    .clone();
    let registry = ComponentRegistryBuilder::new()
        .register(registered)
        .expect("registration")
        .build();
    let component = ComponentName::parse("account.profile").expect("component identity");

    let error = registry
        .require_contract(&component, &other_digest)
        .expect_err("contract mismatch");
    assert_eq!(error.kind(), RegistryErrorKind::ContractMismatch);
    assert_eq!(error.to_string(), "component_contract_mismatch");
    assert!(!format!("{error:?}").contains("account.profile"));
}
