//! Closed action outcome and registered browser emission contracts.

use std::collections::BTreeMap;

use serde::Serialize;
use suprnova_live::action::{
    ActionOutcome, ActionResult, FlashIntent, LiveEffectPayload, LiveEventPayload,
    OutcomeErrorKind, OutcomeMetadata, RegisteredEmission, RouteIntent, UrlIntent,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{BrowserOperationName, ComponentName, RouteIdentity, ViewName};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, EffectMetadata, EffectPayloadMetadata, EventMetadata,
    EventPayloadMetadata,
};
use suprnova_live::registry::ComponentDescriptor;

#[derive(Serialize)]
struct SavedEvent {
    private_note: String,
}

impl EventPayloadMetadata for SavedEvent {
    const NAME: &'static str = "profile.saved";
    const VERSION: u16 = 1;
}

impl LiveEventPayload for SavedEvent {}

#[derive(Serialize)]
struct FocusEffect {
    target: String,
}

impl EffectPayloadMetadata for FocusEffect {
    const NAME: &'static str = "focus";
    const VERSION: u16 = 2;
}

impl LiveEffectPayload for FocusEffect {}

#[derive(Serialize)]
struct UnregisteredEvent;

impl EventPayloadMetadata for UnregisteredEvent {
    const NAME: &'static str = "unregistered";
    const VERSION: u16 = 1;
}

impl LiveEventPayload for UnregisteredEvent {}

#[derive(Serialize)]
struct ForgedSavedEvent;

impl EventPayloadMetadata for ForgedSavedEvent {
    const NAME: &'static str = "profile.saved";
    const VERSION: u16 = 1;
}

impl LiveEventPayload for ForgedSavedEvent {}

fn descriptor() -> ComponentDescriptor {
    let metadata = ComponentMetadata::new_with_browser_contracts(
        ComponentName::parse("profile.editor").expect("component name"),
        ViewName::parse("live/profile/editor.html").expect("view name"),
        ContractVersions::new(1, 1, 1, 1, 2).expect("contract versions"),
        vec![],
        vec![],
        vec![EventMetadata::from_payload::<SavedEvent>().expect("event metadata")],
        vec![EffectMetadata::from_payload::<FocusEffect>().expect("effect metadata")],
        false,
    )
    .expect("component metadata");
    ComponentDescriptor::new(metadata)
}

#[test]
fn registered_events_effects_flash_and_url_intent_are_typed_and_bounded() {
    let descriptor = descriptor();
    let limits = InputLimits::default();
    let event = RegisteredEmission::event(
        &descriptor,
        &SavedEvent {
            private_note: "secret-value".to_owned(),
        },
        &limits,
    )
    .expect("registered event");
    let effect = RegisteredEmission::effect(
        &descriptor,
        &FocusEffect {
            target: "name".to_owned(),
        },
        &limits,
    )
    .expect("registered effect");
    assert!(!format!("{event:?}").contains("secret-value"));
    assert_eq!(effect.name().as_str(), "focus");
    let wrong_channel = OutcomeMetadata::new(vec![], vec![effect.clone()], vec![], None)
        .expect_err("an effect cannot be smuggled through the event channel");
    assert_eq!(
        wrong_channel.kind(),
        OutcomeErrorKind::InvalidEmissionChannel
    );

    let flash = FlashIntent::new(
        BrowserOperationName::parse("profile.saved").expect("flash key"),
        CanonicalValue::String("saved".to_owned()),
        &limits,
    )
    .expect("bounded flash");
    let url = UrlIntent::replace_same_route(
        CanonicalValue::Object(BTreeMap::from([(
            "tab".to_owned(),
            CanonicalValue::String("profile".to_owned()),
        )])),
        &limits,
    )
    .expect("typed URL intent");
    let metadata = OutcomeMetadata::new(vec![flash], vec![event], vec![effect], Some(url))
        .expect("bounded outcome metadata");
    let result = ActionResult::new(ActionOutcome::Render, metadata, &descriptor)
        .expect("compatible render result");
    assert!(result.outcome().requires_render());
}

#[test]
fn redirects_are_real_routes_and_conflicting_or_unregistered_output_is_rejected() {
    let descriptor = descriptor();
    let limits = InputLimits::default();
    let route = RouteIntent::new(
        RouteIdentity::from_bytes(&[0x44; 32]).expect("route identity"),
        CanonicalValue::Object(BTreeMap::new()),
        &limits,
    )
    .expect("safe route intent");
    let reflected = UrlIntent::replace_same_route(CanonicalValue::Object(BTreeMap::new()), &limits)
        .expect("URL reflection");
    let metadata =
        OutcomeMetadata::new(vec![], vec![], vec![], Some(reflected)).expect("outcome metadata");
    let incompatible = ActionResult::new(ActionOutcome::Redirect(route), metadata, &descriptor)
        .expect_err("redirect must suppress incompatible URL reflection");
    assert_eq!(incompatible.kind(), OutcomeErrorKind::IncompatibleOutcome);

    let unregistered = RegisteredEmission::event(&descriptor, &UnregisteredEvent, &limits)
        .expect_err("payload type is not registered by the component");
    assert_eq!(unregistered.kind(), OutcomeErrorKind::UnregisteredEmission);

    let forged = RegisteredEmission::event(&descriptor, &ForgedSavedEvent, &limits)
        .expect_err("matching browser metadata cannot substitute another Rust payload type");
    assert_eq!(forged.kind(), OutcomeErrorKind::UnregisteredEmission);
}
