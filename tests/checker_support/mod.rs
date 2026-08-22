use std::sync::OnceLock;

use suprnova_live::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
use suprnova_live::identity::{ActionName, ComponentName, ModelField, ViewName};
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EffectPayloadMetadata,
    EventMetadata, EventPayloadMetadata, FieldMetadata,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistry, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::state::{BindingTiming, ModelCodec, UrlBinding, UrlBindingMode};
use suprnova_live::validation::ValidationSelection;

pub(crate) const ROOT_VIEW: &str = "tests/root.html";
pub(crate) const CHILD_VIEW: &str = "tests/child.html";

struct ProfileSaved;

impl EventPayloadMetadata for ProfileSaved {
    const NAME: &'static str = "profile-saved";
    const VERSION: u16 = 1;
}

struct FocusSearch;

impl EffectPayloadMetadata for FocusSearch {
    const NAME: &'static str = "focus-search";
    const VERSION: u16 = 1;
}

pub(crate) fn root_name() -> ComponentName {
    ComponentName::parse("tests.root").expect("root component identity")
}

pub(crate) fn child_name() -> ComponentName {
    ComponentName::parse("tests.child").expect("child component identity")
}

pub(crate) fn view(name: &str) -> ViewName {
    ViewName::parse(name).expect("view identity")
}

pub(crate) fn root_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let query = FieldMetadata::new(
            ModelField::parse("query").expect("query field"),
            FieldCategory::Model,
            StateCodec::Json,
            true,
        )
        .with_model_binding(ModelCodec::String, BindingTiming::Blur)
        .expect("query model binding");
        let page = FieldMetadata::new(
            ModelField::parse("page").expect("page field"),
            FieldCategory::State,
            StateCodec::U64Decimal,
            true,
        )
        .with_url_binding(
            UrlBinding::new(
                "page",
                FieldCategory::State,
                ModelCodec::U64,
                UrlBindingMode::Reflect,
                false,
            )
            .expect("page URL binding"),
        )
        .expect("page field URL binding");
        let secret = FieldMetadata::new(
            ModelField::parse("secret").expect("secret field"),
            FieldCategory::Secret,
            StateCodec::Json,
            true,
        );
        ComponentMetadata::new_with_browser_contracts(
            root_name(),
            view(ROOT_VIEW),
            ContractVersions::new(1, 1, 1, 1, 1).expect("root versions"),
            vec![query, page, secret],
            vec![action("refresh"), action("save")],
            vec![EventMetadata::from_payload::<ProfileSaved>().expect("event metadata")],
            vec![EffectMetadata::from_payload::<FocusSearch>().expect("effect metadata")],
            false,
        )
        .expect("root metadata")
    })
}

pub(crate) fn child_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            child_name(),
            view(CHILD_VIEW),
            ContractVersions::new(1, 1, 1, 1, 1).expect("child versions"),
            vec![],
            vec![action("select")],
        )
        .expect("child metadata")
    })
}

pub(crate) fn registry() -> ComponentRegistry {
    ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(root_metadata().clone()))
        .expect("register root")
        .register(ComponentDescriptor::new(child_metadata().clone()))
        .expect("register child")
        .build()
}

fn action(name: &str) -> ActionMetadata {
    ActionMetadata::new_with_contract(
        ActionName::parse(name).expect("action identity"),
        1,
        ActionArgumentSchema::empty(),
        AuthorizationRequirement::Current,
        ValidationSelection::ComponentAndArguments,
        TransactionPolicy::None,
    )
    .expect("action metadata")
}
