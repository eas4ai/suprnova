use suprnova::live::testing::ActionAssertion;
use suprnova::live::{
    ActionOutcome, ActionResult, LiveConfig, LiveConfigErrorKind, LiveRegistry, RegistryErrorKind,
};
use suprnova::view::{TrustedHtml, TrustedMarkupReason};
use suprnova_live::component::ComponentHooks;
use suprnova_live::identity::{ComponentName, ViewName};
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, LiveComponentContract, MetadataError,
};
use suprnova_live::registry::ComponentDescriptor;

fn metadata(component: &str, view: &str) -> ComponentMetadata {
    ComponentMetadata::new(
        ComponentName::parse(component).expect("component identity"),
        ViewName::parse(view).expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 1).expect("contract versions"),
        vec![],
        vec![],
    )
    .expect("component metadata")
}

macro_rules! generated_contract {
    ($name:ident, $component:literal, $view:literal) => {
        struct $name;

        impl LiveComponentContract for $name {
            fn descriptor() -> Result<ComponentDescriptor, MetadataError> {
                Ok(ComponentDescriptor::new(metadata($component, $view)))
            }

            fn descriptor_with_hooks(
                hooks: ComponentHooks,
            ) -> Result<ComponentDescriptor, MetadataError> {
                Ok(ComponentDescriptor::with_hooks(
                    metadata($component, $view),
                    hooks,
                ))
            }
        }
    };
}

generated_contract!(
    GeneratedSearch,
    "catalog.search",
    "live/catalog/search.html"
);
generated_contract!(
    DuplicateSearch,
    "catalog.search",
    "live/catalog/alternate.html"
);
generated_contract!(
    DuplicateView,
    "catalog.alternate",
    "live/catalog/search.html"
);

struct InvalidContract;

impl LiveComponentContract for InvalidContract {
    fn descriptor() -> Result<ComponentDescriptor, MetadataError> {
        ContractVersions::new(0, 1, 1, 1, 1)?;
        unreachable!("invalid versions must fail")
    }

    fn descriptor_with_hooks(_: ComponentHooks) -> Result<ComponentDescriptor, MetadataError> {
        Self::descriptor()
    }
}

#[test]
fn config_is_validated_and_registry_is_immutable_after_build() {
    let config = LiveConfig::builder()
        .max_request_bytes(128 * 1024)
        .max_response_bytes(64 * 1024)
        .build()
        .expect("valid bounded Live configuration");

    assert_eq!(config.max_request_bytes(), 128 * 1024);
    assert_eq!(config.max_response_bytes(), 64 * 1024);

    let invalid = LiveConfig::builder()
        .max_request_bytes(32)
        .max_response_bytes(64)
        .build()
        .expect_err("response ceiling cannot exceed request ceiling");
    assert_eq!(invalid.kind(), LiveConfigErrorKind::InvalidByteLimits);

    let registry = LiveRegistry::builder()
        .register::<GeneratedSearch>()
        .expect("generated contract registers")
        .build();
    assert!(!registry.is_empty());
    assert_eq!(registry.len(), 1);

    let duplicate_component = LiveRegistry::builder()
        .register::<GeneratedSearch>()
        .expect("first component registers")
        .register::<DuplicateSearch>()
        .expect_err("duplicate component identity fails");
    assert_eq!(
        duplicate_component.kind(),
        RegistryErrorKind::DuplicateComponent
    );

    let duplicate_view = LiveRegistry::builder()
        .register::<GeneratedSearch>()
        .expect("first view registers")
        .register::<DuplicateView>()
        .expect_err("duplicate view identity fails");
    assert_eq!(duplicate_view.kind(), RegistryErrorKind::DuplicateView);

    let invalid = LiveRegistry::builder()
        .register::<InvalidContract>()
        .expect_err("invalid generated contract fails");
    assert_eq!(invalid.kind(), RegistryErrorKind::InvalidComponent);

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LiveConfig>();
    assert_send_sync::<LiveRegistry>();
}

#[test]
fn action_view_and_testing_contracts_are_available_from_public_facades() {
    let render = ActionResult::render();
    ActionAssertion::new(&render).assert_rendered();
    assert_eq!(render.outcome(), &ActionOutcome::Render);

    let no_render = ActionResult::no_render();
    ActionAssertion::new(&no_render).assert_not_rendered();
    assert_eq!(no_render.outcome(), &ActionOutcome::NoRender);

    let markup = TrustedHtml::framework_static(
        "<em>framework-owned</em>",
        TrustedMarkupReason::new("facade contract").expect("valid reason"),
    )
    .expect("bounded static markup");
    assert_eq!(markup.to_string(), "<em>framework-owned</em>");
}
