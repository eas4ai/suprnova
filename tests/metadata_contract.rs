//! Canonical component metadata contracts.

use suprnova_live::identity::{ActionName, ComponentName, ModelField, ViewName};
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EffectPayloadMetadata,
    EventMetadata, EventPayloadMetadata, FieldMetadata, MetadataErrorKind,
};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::state::{ModelCodec, ModelPath};
use suprnova_live::validation::ValidationSelection;

fn versions() -> ContractVersions {
    ContractVersions::new(1, 2, 3, 4, 2).expect("valid independent versions")
}

fn field(name: &str, category: FieldCategory) -> FieldMetadata {
    FieldMetadata::new(
        ModelField::parse(name).expect("field identity"),
        category,
        StateCodec::Json,
        true,
    )
}

fn action(name: &str, version: u16) -> ActionMetadata {
    ActionMetadata::new(ActionName::parse(name).expect("action identity"), version)
        .expect("action metadata")
}

fn metadata(fields: Vec<FieldMetadata>, actions: Vec<ActionMetadata>) -> ComponentMetadata {
    ComponentMetadata::new(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        versions(),
        fields,
        actions,
    )
    .expect("component metadata")
}

struct SavedEvent;

impl EventPayloadMetadata for SavedEvent {
    const NAME: &'static str = "saved";
    const VERSION: u16 = 1;
}

struct FocusEffect;

impl EffectPayloadMetadata for FocusEffect {
    const NAME: &'static str = "focus";
    const VERSION: u16 = 2;
}

struct DuplicateSavedEvent;

impl EventPayloadMetadata for DuplicateSavedEvent {
    const NAME: &'static str = "saved";
    const VERSION: u16 = 2;
}

#[test]
fn contract_versions_are_independent_and_nonzero() {
    let versions = versions();

    assert_eq!(versions.component(), 1);
    assert_eq!(versions.state_schema(), 2);
    assert_eq!(versions.action_schema(), 3);
    assert_eq!(versions.checker_contract(), 4);
    assert_eq!(versions.minimum_protocol(), 2);

    let error = ContractVersions::new(1, 2, 0, 4, 2).expect_err("zero action version");
    assert_eq!(error.kind(), MetadataErrorKind::InvalidVersion);

    let unsupported =
        ContractVersions::new(1, 2, 3, 4, 3).expect_err("unsupported minimum protocol");
    assert_eq!(unsupported.kind(), MetadataErrorKind::UnsupportedProtocol);

    let action = ActionMetadata::new(ActionName::parse("save").expect("action identity"), 0)
        .expect_err("zero action version");
    assert_eq!(action.kind(), MetadataErrorKind::InvalidVersion);
}

#[test]
fn view_names_are_bounded_relative_template_identities() {
    assert!(ViewName::parse("components/account/profile.html").is_ok());
    for invalid in [
        "../private.html",
        "/absolute.html",
        "components//profile.html",
        "components/./profile.html",
        "components/../profile.html",
        "components\\profile.html",
    ] {
        assert!(ViewName::parse(invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn duplicate_field_and_action_identities_are_rejected() {
    let duplicate_fields = ComponentMetadata::new(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        versions(),
        vec![
            field("display_name", FieldCategory::Public),
            field("display_name", FieldCategory::Locked),
        ],
        vec![],
    )
    .expect_err("duplicate fields");
    assert_eq!(duplicate_fields.kind(), MetadataErrorKind::DuplicateField);

    let duplicate_actions = ComponentMetadata::new(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        versions(),
        vec![],
        vec![action("save", 1), action("save", 2)],
    )
    .expect_err("duplicate actions");
    assert_eq!(duplicate_actions.kind(), MetadataErrorKind::DuplicateAction);
}

#[test]
fn contract_digest_is_stable_across_metadata_input_order() {
    let first = metadata(
        vec![
            field("display_name", FieldCategory::Public),
            field("account_id", FieldCategory::Locked),
        ],
        vec![action("save", 1), action("reset", 1)],
    );
    let second = metadata(
        vec![
            field("account_id", FieldCategory::Locked),
            field("display_name", FieldCategory::Public),
        ],
        vec![action("reset", 1), action("save", 1)],
    );

    assert_eq!(first.contract_digest(), second.contract_digest());
    assert_ne!(
        first.contract_digest(),
        metadata(
            vec![
                field("display_name", FieldCategory::Public),
                field("account_id", FieldCategory::Locked),
            ],
            vec![action("save", 2), action("reset", 1)],
        )
        .contract_digest()
    );
}

#[test]
fn browser_payload_contracts_are_typed_bounded_and_digest_significant() {
    let saved = EventMetadata::from_payload::<SavedEvent>().expect("event metadata");
    let focus = EffectMetadata::from_payload::<FocusEffect>().expect("effect metadata");
    let with_browser_contracts = ComponentMetadata::new_with_browser_contracts(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        versions(),
        vec![field("display_name", FieldCategory::State)],
        vec![action("save", 1)],
        vec![saved.clone()],
        vec![focus],
        true,
    )
    .expect("browser-aware component metadata");

    assert_eq!(with_browser_contracts.events()[0].name().as_str(), "saved");
    assert_eq!(with_browser_contracts.effects()[0].name().as_str(), "focus");
    assert!(with_browser_contracts.refresh_on_promote());
    assert_ne!(
        with_browser_contracts.contract_digest(),
        metadata(
            vec![field("display_name", FieldCategory::State)],
            vec![action("save", 1)]
        )
        .contract_digest()
    );

    let duplicate = ComponentMetadata::new_with_browser_contracts(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        versions(),
        vec![],
        vec![],
        vec![
            saved,
            EventMetadata::from_payload::<DuplicateSavedEvent>().expect("duplicate metadata"),
        ],
        vec![],
        false,
    )
    .expect_err("duplicate event identity");
    assert_eq!(duplicate.kind(), MetadataErrorKind::DuplicateEvent);
}

#[test]
fn complete_action_dispatch_contract_is_digest_significant() {
    let argument = ActionArgumentField::new(
        ModelField::parse("email").expect("argument identity"),
        ModelCodec::String,
        true,
    )
    .expect("argument contract");
    let protected = ActionMetadata::new_with_contract(
        ActionName::parse("save").expect("action identity"),
        1,
        ActionArgumentSchema::new(vec![argument]).expect("argument schema"),
        AuthorizationRequirement::Current,
        ValidationSelection::Selected(vec![ModelPath::parse("email").expect("validation path")]),
        TransactionPolicy::Required,
    )
    .expect("protected action metadata");
    let public = action("save", 1);

    assert_ne!(
        metadata(vec![], vec![protected]).contract_digest(),
        metadata(vec![], vec![public]).contract_digest()
    );
}

#[test]
fn selected_action_validation_paths_are_canonical_and_unique() {
    let selected = |paths: Vec<ModelPath>| {
        ActionMetadata::new_with_contract(
            ActionName::parse("save").expect("action identity"),
            1,
            ActionArgumentSchema::empty(),
            AuthorizationRequirement::Current,
            ValidationSelection::Selected(paths),
            TransactionPolicy::Required,
        )
    };
    let email = ModelPath::parse("email").expect("email path");
    let name = ModelPath::parse("name").expect("name path");
    let first = selected(vec![name.clone(), email.clone()]).expect("selected metadata");
    let second = selected(vec![email.clone(), name]).expect("selected metadata");
    assert_eq!(
        metadata(vec![], vec![first]).contract_digest(),
        metadata(vec![], vec![second]).contract_digest()
    );

    let duplicate = selected(vec![email.clone(), email]).expect_err("duplicate validation path");
    assert_eq!(duplicate.kind(), MetadataErrorKind::InvalidActionMetadata);
}

#[test]
fn refresh_on_promote_cannot_bypass_the_protocol_v2_contract() {
    let error = ComponentMetadata::new_with_browser_contracts(
        ComponentName::parse("account.profile").expect("component identity"),
        ViewName::parse("components/account/profile.html").expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 1).expect("protocol v1 versions"),
        vec![],
        vec![],
        vec![],
        vec![],
        true,
    )
    .expect_err("refresh-on-promote requires protocol v2");

    assert_eq!(error.kind(), MetadataErrorKind::UnsupportedProtocol);
}
use suprnova_live::action::{
    ActionArgumentField, ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy,
};
