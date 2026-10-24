//! Typed event and stream-subscription metadata contract tests.

use std::num::NonZeroU8;

use suprnova_live::async_updates::{
    BoundedEventNames, BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, MAX_EVENT_FANOUT, MAX_EVENT_TARGETS,
    MAX_SUBSCRIPTION_EVENTS, MAX_SUBSCRIPTION_MODES, MAX_SUBSCRIPTION_TOPICS, MAX_SUBSCRIPTIONS,
    ReconnectPolicy, StreamName, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    TopicName,
};
use suprnova_live::identity::{BrowserOperationName, ComponentName, IslandSlot, ViewName};
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, EventMetadata, EventPayloadMetadata, MetadataError,
    MetadataErrorKind,
};

struct ComponentSaved;

impl EventPayloadMetadata for ComponentSaved {
    const NAME: &'static str = "component_saved";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct StreamSaved;

impl EventPayloadMetadata for StreamSaved {
    const NAME: &'static str = "stream_saved";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct StreamProgress;

impl EventPayloadMetadata for StreamProgress {
    const NAME: &'static str = "stream_progress";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::String;
}

struct PayloadContractAlpha;

impl EventPayloadMetadata for PayloadContractAlpha {
    const NAME: &'static str = "payload_contract_event";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
    const PAYLOAD_CONTRACT: &'static str = "payload_contract_alpha";
}

struct PayloadContractBeta;

impl EventPayloadMetadata for PayloadContractBeta {
    const NAME: &'static str = "payload_contract_event";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
    const PAYLOAD_CONTRACT: &'static str = "payload_contract_beta";
}

fn targets(values: Vec<EventTarget>) -> Result<BoundedTargets, MetadataError> {
    BoundedTargets::new(values)
}

fn event<T: EventPayloadMetadata + 'static>(
    source: EventSource,
    targets: Vec<EventTarget>,
    cycle: EventCyclePolicy,
    maximum_fanout: u16,
) -> Result<EventMetadata, MetadataError> {
    EventMetadata::from_payload_with_contract::<T>(
        source,
        BoundedTargets::new(targets)?,
        EventOrder::PerSourceSequence,
        cycle,
        maximum_fanout,
    )
}

fn topics(values: &[&str]) -> Result<BoundedTopics, MetadataError> {
    BoundedTopics::new(
        values
            .iter()
            .map(|value| TopicName::parse(value))
            .collect::<Result<Vec<_>, _>>()?,
    )
}

fn event_names(values: &[&str]) -> Result<BoundedEventNames, MetadataError> {
    BoundedEventNames::new(
        values
            .iter()
            .map(|value| BrowserOperationName::parse(value).expect("event identity"))
            .collect(),
    )
}

fn modes(values: Vec<SubscriptionMode>) -> Result<SubscriptionModes, MetadataError> {
    SubscriptionModes::new(values)
}

fn subscription(
    stream: &str,
    topics: &[&str],
    events: &[&str],
    modes: Vec<SubscriptionMode>,
    reconnect: ReconnectPolicy,
) -> Result<SubscriptionMetadata, MetadataError> {
    Ok(SubscriptionMetadata::new(
        StreamName::parse(stream)?,
        self::topics(topics)?,
        event_names(events)?,
        self::modes(modes)?,
        reconnect,
    ))
}

fn component(
    events: Vec<EventMetadata>,
    subscriptions: Vec<SubscriptionMetadata>,
) -> Result<ComponentMetadata, MetadataError> {
    ComponentMetadata::new_with_async_contracts(
        ComponentName::parse("tests.async-metadata").expect("component identity"),
        ViewName::parse("tests/async-metadata.html").expect("view identity"),
        ContractVersions::new(1, 1, 1, 1, 2).expect("versions"),
        vec![],
        vec![],
        events,
        vec![],
        subscriptions,
        false,
    )
}

fn stream_component(
    target: EventTarget,
    maximum_fanout: u16,
) -> Result<ComponentMetadata, MetadataError> {
    component(
        vec![event::<StreamSaved>(
            EventSource::Stream,
            vec![target],
            EventCyclePolicy::ForbidRepeatedIsland,
            maximum_fanout,
        )?],
        vec![subscription(
            "account_updates",
            &["tenant/account"],
            &[StreamSaved::NAME],
            vec![SubscriptionMode::ServerSentEvents],
            ReconnectPolicy::ResumeOrRefresh {
                maximum_attempts: NonZeroU8::new(8).expect("nonzero attempts"),
            },
        )?],
    )
}

#[test]
fn component_authored_events_keep_explicit_safe_defaults() {
    let metadata = EventMetadata::from_payload::<ComponentSaved>().expect("event metadata");

    assert_eq!(metadata.name().as_str(), ComponentSaved::NAME);
    assert_eq!(metadata.schema(), BrowserPayloadSchema::Json);
    assert_eq!(metadata.source(), EventSource::Component);
    assert_eq!(metadata.targets().as_slice(), &[EventTarget::SelfIsland]);
    assert_eq!(metadata.order(), EventOrder::PerSourceSequence);
    assert_eq!(metadata.cycle(), EventCyclePolicy::ForbidRepeatedIsland);
    assert_eq!(metadata.maximum_fanout().get(), 1);
}

#[test]
fn explicit_payload_contract_identity_is_component_digest_significant() {
    let alpha = component(
        vec![
            event::<PayloadContractAlpha>(
                EventSource::Component,
                vec![EventTarget::SelfIsland],
                EventCyclePolicy::ForbidRepeatedIsland,
                1,
            )
            .expect("alpha event metadata"),
        ],
        vec![],
    )
    .expect("alpha component");
    let beta = component(
        vec![
            event::<PayloadContractBeta>(
                EventSource::Component,
                vec![EventTarget::SelfIsland],
                EventCyclePolicy::ForbidRepeatedIsland,
                1,
            )
            .expect("beta event metadata"),
        ],
        vec![],
    )
    .expect("beta component");

    assert_eq!(alpha.events()[0].name(), beta.events()[0].name());
    assert_eq!(alpha.events()[0].version(), beta.events()[0].version());
    assert_eq!(alpha.events()[0].schema(), beta.events()[0].schema());
    assert_ne!(
        alpha.events()[0].payload_contract(),
        beta.events()[0].payload_contract()
    );
    assert_ne!(alpha.contract_digest(), beta.contract_digest());
}

#[test]
fn event_target_contract_encodes_named_and_browser_scope_exactly() {
    let named = EventTarget::NamedIsland(
        IslandSlot::parse("sidebar.notifications").expect("named island scope"),
    );
    let browser = EventTarget::Browser(
        BrowserOperationName::parse("application.notifications").expect("browser listener scope"),
    );
    let metadata = event::<ComponentSaved>(
        EventSource::Component,
        vec![browser.clone(), named.clone()],
        EventCyclePolicy::MaximumHops(NonZeroU8::new(3).expect("nonzero hops")),
        2,
    )
    .expect("scoped event metadata");

    assert_eq!(metadata.targets().as_slice(), &[named, browser]);
}

#[test]
fn event_target_scope_cycle_and_fanout_are_digest_significant() {
    let baseline = stream_component(EventTarget::SelfIsland, 8).expect("baseline metadata");
    let document = stream_component(EventTarget::Document, 8).expect("document metadata");
    let named_alpha = stream_component(
        EventTarget::NamedIsland(IslandSlot::parse("alpha").expect("alpha slot")),
        8,
    )
    .expect("named alpha metadata");
    let named_beta = stream_component(
        EventTarget::NamedIsland(IslandSlot::parse("beta").expect("beta slot")),
        8,
    )
    .expect("named beta metadata");
    let higher_fanout = stream_component(EventTarget::SelfIsland, 9).expect("fanout metadata");
    let bounded_cycle = component(
        vec![
            event::<StreamSaved>(
                EventSource::Stream,
                vec![EventTarget::SelfIsland],
                EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("nonzero hops")),
                8,
            )
            .expect("cycle metadata"),
        ],
        baseline.subscriptions().to_vec(),
    )
    .expect("bounded-cycle component");
    assert_ne!(baseline.contract_digest(), document.contract_digest());
    assert_ne!(named_alpha.contract_digest(), named_beta.contract_digest());
    assert_ne!(baseline.contract_digest(), higher_fanout.contract_digest());
    assert_ne!(baseline.contract_digest(), bounded_cycle.contract_digest());
}

#[test]
fn event_targets_and_fanout_reject_duplicate_or_unbounded_contracts() {
    let duplicate = targets(vec![EventTarget::SelfIsland, EventTarget::SelfIsland])
        .expect_err("duplicate target");
    assert_eq!(duplicate.kind(), MetadataErrorKind::DuplicateEventTarget);

    let unbounded = (0..=MAX_EVENT_TARGETS)
        .map(|index| {
            EventTarget::NamedIsland(
                IslandSlot::parse(&format!("island-{index}")).expect("named island scope"),
            )
        })
        .collect();
    let too_many = targets(unbounded).expect_err("unbounded targets");
    assert_eq!(too_many.kind(), MetadataErrorKind::TooManyEventTargets);

    let zero = event::<ComponentSaved>(
        EventSource::Component,
        vec![EventTarget::SelfIsland],
        EventCyclePolicy::ForbidRepeatedIsland,
        0,
    )
    .expect_err("zero fanout");
    assert_eq!(zero.kind(), MetadataErrorKind::InvalidEventFanout);

    let too_large = event::<ComponentSaved>(
        EventSource::Component,
        vec![EventTarget::SelfIsland],
        EventCyclePolicy::ForbidRepeatedIsland,
        MAX_EVENT_FANOUT + 1,
    )
    .expect_err("unbounded fanout");
    assert_eq!(too_large.kind(), MetadataErrorKind::InvalidEventFanout);

    let below_target_count = event::<ComponentSaved>(
        EventSource::Component,
        vec![EventTarget::SelfIsland, EventTarget::Document],
        EventCyclePolicy::ForbidRepeatedIsland,
        1,
    )
    .expect_err("fanout below target count");
    assert_eq!(
        below_target_count.kind(),
        MetadataErrorKind::InvalidEventFanout
    );
}

#[test]
fn empty_async_collections_are_rejected() {
    let cases = [
        (
            "targets",
            BoundedTargets::new(vec![])
                .expect_err("empty targets")
                .kind(),
            MetadataErrorKind::InvalidEventTarget,
        ),
        (
            "topics",
            BoundedTopics::new(vec![]).expect_err("empty topics").kind(),
            MetadataErrorKind::InvalidSubscriptionMetadata,
        ),
        (
            "events",
            BoundedEventNames::new(vec![])
                .expect_err("empty events")
                .kind(),
            MetadataErrorKind::InvalidSubscriptionMetadata,
        ),
        (
            "modes",
            SubscriptionModes::new(vec![])
                .expect_err("empty modes")
                .kind(),
            MetadataErrorKind::InvalidSubscriptionMetadata,
        ),
    ];

    for (collection, actual, expected) in cases {
        assert_eq!(actual, expected, "{collection}");
    }
}

#[test]
fn exact_async_collection_maxima_are_accepted() {
    let maximum_targets = BoundedTargets::new(
        (0..MAX_EVENT_TARGETS)
            .map(|index| {
                EventTarget::NamedIsland(
                    IslandSlot::parse(&format!("island-{index}")).expect("named island scope"),
                )
            })
            .collect(),
    )
    .expect("maximum targets");
    let maximum_topics = BoundedTopics::new(
        (0..MAX_SUBSCRIPTION_TOPICS)
            .map(|index| TopicName::parse(&format!("tenant/{index}")))
            .collect::<Result<Vec<_>, _>>()
            .expect("topic identities"),
    )
    .expect("maximum topics");
    let maximum_events = BoundedEventNames::new(
        (0..MAX_SUBSCRIPTION_EVENTS)
            .map(|index| BrowserOperationName::parse(&format!("stream_event_{index}")))
            .collect::<Result<Vec<_>, _>>()
            .expect("event identities"),
    )
    .expect("maximum events");
    let maximum_modes = SubscriptionModes::new(vec![
        SubscriptionMode::ServerSentEvents,
        SubscriptionMode::WebSocket,
    ])
    .expect("maximum modes");

    assert_eq!(maximum_targets.as_slice().len(), MAX_EVENT_TARGETS);
    assert_eq!(maximum_topics.as_slice().len(), MAX_SUBSCRIPTION_TOPICS);
    assert_eq!(maximum_events.as_slice().len(), MAX_SUBSCRIPTION_EVENTS);
    assert_eq!(maximum_modes.as_slice().len(), MAX_SUBSCRIPTION_MODES);
}

#[test]
fn subscription_declarations_are_sorted_typed_and_digest_significant() {
    let declaration = subscription(
        "account_updates",
        &["tenant/z", "tenant/a"],
        &[StreamSaved::NAME, StreamProgress::NAME],
        vec![
            SubscriptionMode::WebSocket,
            SubscriptionMode::ServerSentEvents,
        ],
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(5).expect("nonzero attempts"),
        },
    )
    .expect("subscription metadata");

    assert_eq!(declaration.stream().as_str(), "account_updates");
    assert_eq!(
        declaration
            .topics()
            .as_slice()
            .iter()
            .map(TopicName::as_str)
            .collect::<Vec<_>>(),
        vec!["tenant/a", "tenant/z"]
    );
    assert_eq!(
        declaration
            .events()
            .as_slice()
            .iter()
            .map(BrowserOperationName::as_str)
            .collect::<Vec<_>>(),
        vec![StreamProgress::NAME, StreamSaved::NAME]
    );
    assert_eq!(
        declaration.modes().as_slice(),
        &[
            SubscriptionMode::ServerSentEvents,
            SubscriptionMode::WebSocket,
        ]
    );

    let baseline = stream_component(EventTarget::SelfIsland, 8).expect("baseline metadata");
    let changed_stream = component(
        baseline.events().to_vec(),
        vec![
            subscription(
                "other_updates",
                &["tenant/account"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::ResumeOrRefresh {
                    maximum_attempts: NonZeroU8::new(8).expect("nonzero attempts"),
                },
            )
            .expect("changed stream"),
        ],
    )
    .expect("changed stream component");
    let changed_topic = component(
        baseline.events().to_vec(),
        vec![
            subscription(
                "account_updates",
                &["tenant/other"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::ResumeOrRefresh {
                    maximum_attempts: NonZeroU8::new(8).expect("nonzero attempts"),
                },
            )
            .expect("changed topic"),
        ],
    )
    .expect("changed topic component");
    let changed_mode = component(
        baseline.events().to_vec(),
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::WebSocket],
                ReconnectPolicy::ResumeOrRefresh {
                    maximum_attempts: NonZeroU8::new(8).expect("nonzero attempts"),
                },
            )
            .expect("changed mode"),
        ],
    )
    .expect("changed mode component");
    let changed_reconnect = component(
        baseline.events().to_vec(),
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("changed reconnect"),
        ],
    )
    .expect("changed reconnect component");

    assert_ne!(baseline.contract_digest(), changed_stream.contract_digest());
    assert_ne!(baseline.contract_digest(), changed_topic.contract_digest());
    assert_ne!(baseline.contract_digest(), changed_mode.contract_digest());
    assert_ne!(
        baseline.contract_digest(),
        changed_reconnect.contract_digest()
    );
}

#[test]
fn subscription_events_are_independently_digest_significant() {
    let registered_events = vec![
        event::<StreamSaved>(
            EventSource::Stream,
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            8,
        )
        .expect("saved event metadata"),
        event::<StreamProgress>(
            EventSource::Stream,
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            8,
        )
        .expect("progress event metadata"),
    ];
    let progress_subscription = subscription(
        "progress_updates",
        &["tenant/account"],
        &[StreamProgress::NAME],
        vec![SubscriptionMode::ServerSentEvents],
        ReconnectPolicy::RefreshOnReconnect,
    )
    .expect("progress subscription");
    let baseline = component(
        registered_events.clone(),
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("baseline subscription"),
            progress_subscription.clone(),
        ],
    )
    .expect("baseline component");
    let changed_events = component(
        registered_events,
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &[StreamSaved::NAME, StreamProgress::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("changed event declaration"),
            progress_subscription,
        ],
    )
    .expect("changed component");

    assert_ne!(baseline.contract_digest(), changed_events.contract_digest());
}

#[test]
fn component_subscription_input_order_does_not_change_the_digest() {
    let registered_events = vec![
        event::<StreamSaved>(
            EventSource::Stream,
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            8,
        )
        .expect("saved event metadata"),
        event::<StreamProgress>(
            EventSource::Stream,
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            8,
        )
        .expect("progress event metadata"),
    ];
    let saved = subscription(
        "account_updates",
        &["tenant/account"],
        &[StreamSaved::NAME],
        vec![SubscriptionMode::ServerSentEvents],
        ReconnectPolicy::RefreshOnReconnect,
    )
    .expect("saved subscription");
    let progress = subscription(
        "progress_updates",
        &["tenant/account"],
        &[StreamProgress::NAME],
        vec![SubscriptionMode::WebSocket],
        ReconnectPolicy::RefreshOnReconnect,
    )
    .expect("progress subscription");

    let forward = component(
        registered_events.clone(),
        vec![saved.clone(), progress.clone()],
    )
    .expect("forward component");
    let reversed = component(registered_events, vec![progress, saved]).expect("reversed component");

    assert_eq!(forward.contract_digest(), reversed.contract_digest());
    assert_eq!(forward.subscriptions(), reversed.subscriptions());
}

#[test]
fn subscription_collections_reject_duplicate_and_unbounded_contracts() {
    let duplicate_topic =
        topics(&["tenant/account", "tenant/account"]).expect_err("duplicate subscription topic");
    assert_eq!(
        duplicate_topic.kind(),
        MetadataErrorKind::DuplicateSubscriptionTopic
    );

    let many_topics = (0..=MAX_SUBSCRIPTION_TOPICS)
        .map(|index| TopicName::parse(&format!("tenant/{index}")))
        .collect::<Result<Vec<_>, _>>()
        .expect("bounded topic identities");
    let too_many_topics = BoundedTopics::new(many_topics).expect_err("unbounded topics");
    assert_eq!(
        too_many_topics.kind(),
        MetadataErrorKind::TooManySubscriptionTopics
    );

    let duplicate_event = event_names(&[StreamSaved::NAME, StreamSaved::NAME])
        .expect_err("duplicate subscription event");
    assert_eq!(
        duplicate_event.kind(),
        MetadataErrorKind::DuplicateSubscriptionEvent
    );

    let many_events = (0..=MAX_SUBSCRIPTION_EVENTS)
        .map(|index| BrowserOperationName::parse(&format!("stream_event_{index}")))
        .collect::<Result<Vec<_>, _>>()
        .expect("bounded event identities");
    let too_many_events = BoundedEventNames::new(many_events).expect_err("unbounded events");
    assert_eq!(
        too_many_events.kind(),
        MetadataErrorKind::TooManySubscriptionEvents
    );

    let duplicate_mode = modes(vec![
        SubscriptionMode::WebSocket,
        SubscriptionMode::WebSocket,
    ])
    .expect_err("duplicate subscription mode");
    assert_eq!(
        duplicate_mode.kind(),
        MetadataErrorKind::DuplicateSubscriptionMode
    );

    let too_many_modes = modes(vec![
        SubscriptionMode::ServerSentEvents;
        MAX_SUBSCRIPTION_MODES + 1
    ])
    .expect_err("unbounded subscription modes");
    assert_eq!(
        too_many_modes.kind(),
        MetadataErrorKind::TooManySubscriptionModes
    );
}

#[test]
fn component_rejects_duplicate_subscription_declarations() {
    let stream_event = event::<StreamSaved>(
        EventSource::Stream,
        vec![EventTarget::SelfIsland],
        EventCyclePolicy::ForbidRepeatedIsland,
        8,
    )
    .expect("stream event metadata");
    let declaration = subscription(
        "account_updates",
        &["tenant/account"],
        &[StreamSaved::NAME],
        vec![SubscriptionMode::ServerSentEvents],
        ReconnectPolicy::RefreshOnReconnect,
    )
    .expect("subscription metadata");

    let duplicate = component(
        vec![stream_event.clone()],
        vec![declaration.clone(), declaration],
    )
    .expect_err("duplicate subscription");
    assert_eq!(duplicate.kind(), MetadataErrorKind::DuplicateSubscription);
}

#[test]
fn component_rejects_unbounded_subscription_declarations() {
    let stream_event = event::<StreamSaved>(
        EventSource::Stream,
        vec![EventTarget::SelfIsland],
        EventCyclePolicy::ForbidRepeatedIsland,
        8,
    )
    .expect("stream event metadata");
    let subscriptions = (0..=MAX_SUBSCRIPTIONS)
        .map(|index| {
            subscription(
                &format!("stream_{index}"),
                &["tenant/account"],
                &[StreamSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("subscription metadata")
        })
        .collect();
    let unbounded = component(vec![stream_event], subscriptions)
        .expect_err("unbounded component subscriptions");
    assert_eq!(unbounded.kind(), MetadataErrorKind::TooManySubscriptions);
}

#[test]
fn component_requires_every_subscription_event_to_be_a_registered_stream_event() {
    let stream_event = event::<StreamSaved>(
        EventSource::Stream,
        vec![EventTarget::SelfIsland],
        EventCyclePolicy::ForbidRepeatedIsland,
        8,
    )
    .expect("stream event metadata");

    let missing_subscription =
        component(vec![stream_event.clone()], vec![]).expect_err("unregistered stream event");
    assert_eq!(
        missing_subscription.kind(),
        MetadataErrorKind::UnregisteredStreamEvent
    );

    let unknown_event = component(
        vec![stream_event],
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &["unknown_event"],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("subscription metadata"),
        ],
    )
    .expect_err("unknown subscription event");
    assert_eq!(
        unknown_event.kind(),
        MetadataErrorKind::UnknownSubscriptionEvent
    );

    let component_event = EventMetadata::from_payload::<ComponentSaved>().expect("component event");
    let wrong_source = component(
        vec![component_event],
        vec![
            subscription(
                "account_updates",
                &["tenant/account"],
                &[ComponentSaved::NAME],
                vec![SubscriptionMode::ServerSentEvents],
                ReconnectPolicy::RefreshOnReconnect,
            )
            .expect("subscription metadata"),
        ],
    )
    .expect_err("component event cannot be a stream declaration");
    assert_eq!(
        wrong_source.kind(),
        MetadataErrorKind::UnknownSubscriptionEvent
    );
}
