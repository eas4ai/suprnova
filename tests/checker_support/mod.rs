use std::num::NonZeroU8;
use std::sync::OnceLock;

use suprnova_live::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
use suprnova_live::async_updates::{
    BoundedEventNames, BoundedTargets, BoundedTopics, EventCyclePolicy, EventOrder, EventSource,
    EventTarget, ReconnectPolicy, StreamName, SubscriptionMetadata, SubscriptionMode,
    SubscriptionModes, TopicName,
};
use suprnova_live::identity::{
    ActionName, BrowserOperationName, ComponentName, ModelField, ViewName,
};
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EffectPayloadMetadata,
    EventMetadata, EventPayloadMetadata, FieldMetadata,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistry, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::state::{BindingTiming, ModelCodec, UrlBinding, UrlBindingMode};
use suprnova_live::upload::{
    UploadFieldPolicy, UploadMediaType, UploadReplacementPolicy, UploadScanPolicy,
};
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

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
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

pub(crate) fn registry_with_checker_contract(checker_contract: u16) -> ComponentRegistry {
    ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(
            root_metadata_with_checker_contract(checker_contract),
        ))
        .expect("register root")
        .register(ComponentDescriptor::new(child_metadata().clone()))
        .expect("register child")
        .build()
}

fn root_metadata_with_checker_contract(checker_contract: u16) -> ComponentMetadata {
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
    let avatar = FieldMetadata::new(
        ModelField::parse("avatar").expect("avatar field"),
        FieldCategory::Model,
        StateCodec::Json,
        true,
    )
    .with_model_binding(ModelCodec::String, BindingTiming::Change)
    .expect("avatar model binding")
    .with_upload_policy(
        UploadFieldPolicy::new(
            4,
            4 * 1024 * 1024,
            UploadReplacementPolicy::RetirePrevious,
            vec![UploadMediaType::Png, UploadMediaType::Jpeg],
            None,
            UploadScanPolicy::Disabled,
            ActionName::parse("save").expect("upload finalize action"),
        )
        .expect("avatar upload policy"),
    )
    .expect("avatar upload metadata");
    ComponentMetadata::new_with_async_contracts(
        root_name(),
        view(ROOT_VIEW),
        ContractVersions::new(1, 1, 1, checker_contract, 1).expect("root versions"),
        vec![query, page, secret, avatar],
        vec![action("refresh"), action("save")],
        vec![
            EventMetadata::from_payload::<ProfileSaved>().expect("event metadata"),
            orders_event_metadata(),
        ],
        vec![EffectMetadata::from_payload::<FocusSearch>().expect("effect metadata")],
        vec![orders_subscription_metadata()],
        false,
    )
    .expect("root metadata")
}

pub(crate) fn child_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new_with_async_contracts(
            child_name(),
            view(CHILD_VIEW),
            ContractVersions::new(1, 1, 1, 2, 1).expect("child versions"),
            vec![],
            vec![action("select")],
            vec![orders_event_metadata()],
            vec![],
            vec![orders_subscription_metadata()],
            false,
        )
        .expect("child metadata")
    })
}

pub(crate) fn registry() -> ComponentRegistry {
    registry_with_checker_contract(2)
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

fn orders_event_metadata() -> EventMetadata {
    EventMetadata::from_payload_with_contract::<OrdersUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("orders targets"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::ForbidRepeatedIsland,
        1,
    )
    .expect("orders event metadata")
}

fn orders_subscription_metadata() -> SubscriptionMetadata {
    SubscriptionMetadata::new(
        StreamName::parse("orders").expect("orders stream"),
        BoundedTopics::new(vec![
            TopicName::parse("tenant/orders").expect("orders topic"),
        ])
        .expect("orders topics"),
        BoundedEventNames::new(vec![
            BrowserOperationName::parse(OrdersUpdated::NAME).expect("orders event identity"),
        ])
        .expect("orders events"),
        SubscriptionModes::new(vec![
            SubscriptionMode::ServerSentEvents,
            SubscriptionMode::WebSocket,
        ])
        .expect("orders modes"),
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(3).expect("nonzero reconnect attempts"),
        },
    )
}
