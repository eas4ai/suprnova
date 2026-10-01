//! Purpose-specific canonical component-contract digest.

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};

use crate::action::{AuthorizationRequirement, TransactionPolicy};
use crate::async_updates::{
    BrowserPayloadSchema, EventCyclePolicy, EventOrder, EventSource, EventTarget, ReconnectPolicy,
    SubscriptionMetadata, SubscriptionMode,
};
use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{ComponentName, ContentDigest, ViewName};
use crate::limits::InputLimits;
use crate::snapshot::state::{FieldCategory, StateCodec};
use crate::state::{BindingTiming, ModelCodec, UrlBindingMode};
use crate::upload::{
    ScanFailurePolicy, UploadFieldPolicy, UploadReplacementPolicy, UploadScanPolicy,
};
use crate::validation::ValidationSelection;

use super::{
    ActionMetadata, ContractVersions, EffectMetadata, EventMetadata, FieldMetadata, MetadataError,
    MetadataErrorKind,
};

const CONTRACT_DIGEST_DOMAIN: &[u8] = b"suprnova-live/component-contract/v1";

#[allow(
    clippy::too_many_arguments,
    reason = "the digest intentionally receives every independent semantic contract dimension"
)]
pub(super) fn contract_digest(
    identity: &ComponentName,
    view: &ViewName,
    versions: ContractVersions,
    fields: &[FieldMetadata],
    actions: &[ActionMetadata],
    events: &[EventMetadata],
    effects: &[EffectMetadata],
    subscriptions: &[SubscriptionMetadata],
    refresh_on_promote: bool,
) -> Result<ContentDigest, MetadataError> {
    let value = CanonicalValue::Object(BTreeMap::from([
        (
            "actions".to_owned(),
            CanonicalValue::Array(actions.iter().map(action_value).collect()),
        ),
        (
            "component".to_owned(),
            CanonicalValue::String(identity.as_str().to_owned()),
        ),
        (
            "effects".to_owned(),
            CanonicalValue::Array(effects.iter().map(effect_value).collect()),
        ),
        (
            "events".to_owned(),
            CanonicalValue::Array(events.iter().map(event_value).collect()),
        ),
        (
            "fields".to_owned(),
            CanonicalValue::Array(fields.iter().map(field_value).collect()),
        ),
        (
            "refresh_on_promote".to_owned(),
            CanonicalValue::Bool(refresh_on_promote),
        ),
        (
            "subscriptions".to_owned(),
            CanonicalValue::Array(subscriptions.iter().map(subscription_value).collect()),
        ),
        (
            "versions".to_owned(),
            CanonicalValue::Object(BTreeMap::from([
                version_entry("action_schema", versions.action_schema()),
                version_entry("checker_contract", versions.checker_contract()),
                version_entry("component", versions.component()),
                version_entry("minimum_protocol", versions.minimum_protocol()),
                version_entry("state_schema", versions.state_schema()),
            ])),
        ),
        (
            "view".to_owned(),
            CanonicalValue::String(view.as_str().to_owned()),
        ),
    ]));
    let limits = InputLimits::new(256 * 1024, 16, 8_192, 1_024)
        .map_err(|_| MetadataError::new(MetadataErrorKind::ContractEncodingFailed))?;
    let encoded = to_canonical_bytes(&value, &limits)
        .map_err(|_| MetadataError::new(MetadataErrorKind::ContractEncodingFailed))?;
    let mut hasher = Sha256::new();
    hasher.update(CONTRACT_DIGEST_DOMAIN);
    hasher.update(encoded);
    ContentDigest::from_bytes(&hasher.finalize())
        .map_err(|_| MetadataError::new(MetadataErrorKind::ContractEncodingFailed))
}

fn version_entry(name: &str, version: u16) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(version.to_string()))
}

fn field_value(field: &FieldMetadata) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "binding_timing".to_owned(),
            timing_value(field.binding_timing()),
        ),
        (
            "category".to_owned(),
            CanonicalValue::String(category_name(field.category()).to_owned()),
        ),
        (
            "codec".to_owned(),
            CanonicalValue::String(codec_name(field.codec()).to_owned()),
        ),
        (
            "model_codec".to_owned(),
            field
                .model_codec()
                .map_or(CanonicalValue::Null, model_codec_value),
        ),
        (
            "name".to_owned(),
            CanonicalValue::String(field.name().as_str().to_owned()),
        ),
        (
            "required".to_owned(),
            CanonicalValue::Bool(field.required()),
        ),
        (
            "session_codec".to_owned(),
            field
                .session_codec()
                .map_or(CanonicalValue::Null, model_codec_value),
        ),
        (
            "url".to_owned(),
            field.url_binding().map_or(CanonicalValue::Null, |binding| {
                CanonicalValue::Object(BTreeMap::from([
                    ("codec".to_owned(), model_codec_value(binding.codec())),
                    (
                        "mode".to_owned(),
                        CanonicalValue::String(
                            match binding.mode() {
                                UrlBindingMode::Reflect => "reflect",
                                UrlBindingMode::Navigate => "navigate",
                            }
                            .to_owned(),
                        ),
                    ),
                    (
                        "omit_default".to_owned(),
                        CanonicalValue::Bool(binding.omit_default()),
                    ),
                    (
                        "query_key".to_owned(),
                        CanonicalValue::String(binding.query_key().to_owned()),
                    ),
                ]))
            }),
        ),
        (
            "upload".to_owned(),
            field
                .upload_policy()
                .map_or(CanonicalValue::Null, upload_policy_value),
        ),
    ]))
}

fn upload_policy_value(policy: &UploadFieldPolicy) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "accepted_types".to_owned(),
            CanonicalValue::Array(
                policy
                    .accepted_types()
                    .iter()
                    .map(|accepted| {
                        CanonicalValue::Object(BTreeMap::from([
                            (
                                "extensions".to_owned(),
                                CanonicalValue::Array(
                                    accepted
                                        .extensions()
                                        .iter()
                                        .cloned()
                                        .map(CanonicalValue::String)
                                        .collect(),
                                ),
                            ),
                            (
                                "media_type".to_owned(),
                                CanonicalValue::String(accepted.media_type().to_owned()),
                            ),
                        ]))
                    })
                    .collect(),
            ),
        ),
        (
            "dimensions".to_owned(),
            policy.dimensions().map_or(CanonicalValue::Null, |limits| {
                CanonicalValue::Object(BTreeMap::from([
                    (
                        "maximum_height".to_owned(),
                        CanonicalValue::String(limits.maximum_height().to_string()),
                    ),
                    (
                        "maximum_pixels".to_owned(),
                        CanonicalValue::String(limits.maximum_pixels().to_string()),
                    ),
                    (
                        "maximum_width".to_owned(),
                        CanonicalValue::String(limits.maximum_width().to_string()),
                    ),
                ]))
            }),
        ),
        (
            "finalize_action".to_owned(),
            CanonicalValue::String(policy.finalize_action().as_str().to_owned()),
        ),
        (
            "maximum_file_bytes".to_owned(),
            CanonicalValue::String(policy.maximum_file_bytes().to_string()),
        ),
        (
            "maximum_files".to_owned(),
            CanonicalValue::String(policy.maximum_files().to_string()),
        ),
        (
            "replacement".to_owned(),
            CanonicalValue::String(
                match policy.replacement() {
                    UploadReplacementPolicy::RetirePrevious => "retire_previous",
                    UploadReplacementPolicy::PreservePrevious => "preserve_previous",
                }
                .to_owned(),
            ),
        ),
        ("scan".to_owned(), scan_policy_value(policy.scan())),
    ]))
}

fn scan_policy_value(policy: UploadScanPolicy) -> CanonicalValue {
    match policy {
        UploadScanPolicy::Disabled => CanonicalValue::String("disabled".to_owned()),
        UploadScanPolicy::Required {
            on_timeout,
            on_unavailable,
        } => CanonicalValue::Object(BTreeMap::from([
            (
                "on_timeout".to_owned(),
                CanonicalValue::String(scan_failure_name(on_timeout).to_owned()),
            ),
            (
                "on_unavailable".to_owned(),
                CanonicalValue::String(scan_failure_name(on_unavailable).to_owned()),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("required".to_owned()),
            ),
        ])),
    }
}

const fn scan_failure_name(policy: ScanFailurePolicy) -> &'static str {
    match policy {
        ScanFailurePolicy::Retry => "retry",
        ScanFailurePolicy::Reject => "reject",
    }
}

fn timing_value(timing: Option<BindingTiming>) -> CanonicalValue {
    match timing {
        None => CanonicalValue::Null,
        Some(BindingTiming::Immediate) => CanonicalValue::String("immediate".to_owned()),
        Some(BindingTiming::Change) => CanonicalValue::String("change".to_owned()),
        Some(BindingTiming::Blur) => CanonicalValue::String("blur".to_owned()),
        Some(BindingTiming::Submit) => CanonicalValue::String("submit".to_owned()),
        Some(BindingTiming::Debounce(milliseconds)) => CanonicalValue::Object(BTreeMap::from([
            (
                "milliseconds".to_owned(),
                CanonicalValue::String(milliseconds.to_string()),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("debounce".to_owned()),
            ),
        ])),
    }
}

fn model_codec_value(codec: &ModelCodec) -> CanonicalValue {
    match codec {
        ModelCodec::String => CanonicalValue::String("string".to_owned()),
        ModelCodec::Boolean => CanonicalValue::String("boolean".to_owned()),
        ModelCodec::I64 => CanonicalValue::String("i64".to_owned()),
        ModelCodec::U64 => CanonicalValue::String("u64".to_owned()),
        ModelCodec::F64 => CanonicalValue::String("f64".to_owned()),
        ModelCodec::Json => CanonicalValue::String("json".to_owned()),
        ModelCodec::Date => CanonicalValue::String("date".to_owned()),
        ModelCodec::DateTime => CanonicalValue::String("datetime".to_owned()),
        ModelCodec::Uuid => CanonicalValue::String("uuid".to_owned()),
        ModelCodec::Enumeration(variants) => CanonicalValue::Object(BTreeMap::from([
            ("type".to_owned(), CanonicalValue::String("enum".to_owned())),
            (
                "variants".to_owned(),
                CanonicalValue::Array(
                    variants
                        .iter()
                        .cloned()
                        .map(CanonicalValue::String)
                        .collect(),
                ),
            ),
        ])),
        ModelCodec::List(entry) => CanonicalValue::Object(BTreeMap::from([
            ("entry".to_owned(), model_codec_value(entry)),
            ("type".to_owned(), CanonicalValue::String("list".to_owned())),
        ])),
        ModelCodec::Map(value) => CanonicalValue::Object(BTreeMap::from([
            ("type".to_owned(), CanonicalValue::String("map".to_owned())),
            ("value".to_owned(), model_codec_value(value)),
        ])),
    }
}

fn action_value(action: &ActionMetadata) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "arguments".to_owned(),
            CanonicalValue::Array(
                action
                    .arguments()
                    .fields()
                    .map(|field| {
                        CanonicalValue::Object(BTreeMap::from([
                            ("codec".to_owned(), model_codec_value(field.codec())),
                            (
                                "name".to_owned(),
                                CanonicalValue::String(field.name().as_str().to_owned()),
                            ),
                            (
                                "required".to_owned(),
                                CanonicalValue::Bool(field.required()),
                            ),
                        ]))
                    })
                    .collect(),
            ),
        ),
        (
            "authorization".to_owned(),
            CanonicalValue::String(
                match action.authorization() {
                    AuthorizationRequirement::Public => "public",
                    AuthorizationRequirement::Current => "current",
                }
                .to_owned(),
            ),
        ),
        (
            "name".to_owned(),
            CanonicalValue::String(action.name().as_str().to_owned()),
        ),
        (
            "transaction".to_owned(),
            CanonicalValue::String(
                match action.transaction() {
                    TransactionPolicy::None => "none",
                    TransactionPolicy::Required => "required",
                }
                .to_owned(),
            ),
        ),
        (
            "validation".to_owned(),
            validation_selection_value(action.validation()),
        ),
        (
            "version".to_owned(),
            CanonicalValue::String(action.version().to_string()),
        ),
    ]))
}

fn validation_selection_value(selection: &ValidationSelection) -> CanonicalValue {
    match selection {
        ValidationSelection::None => CanonicalValue::String("none".to_owned()),
        ValidationSelection::WholeComponent => CanonicalValue::String("whole_component".to_owned()),
        ValidationSelection::ActionArguments => {
            CanonicalValue::String("action_arguments".to_owned())
        }
        ValidationSelection::ComponentAndArguments => {
            CanonicalValue::String("component_and_arguments".to_owned())
        }
        ValidationSelection::Selected(paths) => CanonicalValue::Object(BTreeMap::from([
            (
                "paths".to_owned(),
                CanonicalValue::Array(
                    paths
                        .iter()
                        .map(|path| CanonicalValue::String(path.as_str().to_owned()))
                        .collect(),
                ),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("selected".to_owned()),
            ),
        ])),
    }
}

fn event_value(event: &EventMetadata) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("cycle".to_owned(), event_cycle_value(event.cycle())),
        (
            "maximum_fanout".to_owned(),
            CanonicalValue::String(event.maximum_fanout().to_string()),
        ),
        (
            "name".to_owned(),
            CanonicalValue::String(event.name().as_str().to_owned()),
        ),
        (
            "order".to_owned(),
            CanonicalValue::String(event_order_name(event.order()).to_owned()),
        ),
        (
            "payload_contract".to_owned(),
            CanonicalValue::String(event.payload_contract().as_str().to_owned()),
        ),
        (
            "schema".to_owned(),
            CanonicalValue::String(payload_schema_name(event.schema()).to_owned()),
        ),
        (
            "source".to_owned(),
            CanonicalValue::String(event_source_name(event.source()).to_owned()),
        ),
        (
            "targets".to_owned(),
            CanonicalValue::Array(
                event
                    .targets()
                    .as_slice()
                    .iter()
                    .map(event_target_value)
                    .collect(),
            ),
        ),
        (
            "version".to_owned(),
            CanonicalValue::String(event.version().to_string()),
        ),
    ]))
}

fn effect_value(effect: &EffectMetadata) -> CanonicalValue {
    browser_operation_value(effect.name().as_str(), effect.version())
}

fn subscription_value(subscription: &SubscriptionMetadata) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "events".to_owned(),
            CanonicalValue::Array(
                subscription
                    .events()
                    .as_slice()
                    .iter()
                    .map(|event| CanonicalValue::String(event.as_str().to_owned()))
                    .collect(),
            ),
        ),
        (
            "modes".to_owned(),
            CanonicalValue::Array(
                subscription
                    .modes()
                    .as_slice()
                    .iter()
                    .map(|mode| CanonicalValue::String(subscription_mode_name(*mode).to_owned()))
                    .collect(),
            ),
        ),
        (
            "reconnect".to_owned(),
            reconnect_value(subscription.reconnect()),
        ),
        (
            "stream".to_owned(),
            CanonicalValue::String(subscription.stream().as_str().to_owned()),
        ),
        (
            "topics".to_owned(),
            CanonicalValue::Array(
                subscription
                    .topics()
                    .as_slice()
                    .iter()
                    .map(|topic| CanonicalValue::String(topic.as_str().to_owned()))
                    .collect(),
            ),
        ),
    ]))
}

fn event_target_value(target: &EventTarget) -> CanonicalValue {
    match target {
        EventTarget::SelfIsland => CanonicalValue::String("self_island".to_owned()),
        EventTarget::Parent => CanonicalValue::String("parent".to_owned()),
        EventTarget::Child => CanonicalValue::String("child".to_owned()),
        EventTarget::NamedIsland(slot) => CanonicalValue::Object(BTreeMap::from([
            (
                "slot".to_owned(),
                CanonicalValue::String(slot.as_str().to_owned()),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("named_island".to_owned()),
            ),
        ])),
        EventTarget::Document => CanonicalValue::String("document".to_owned()),
        EventTarget::Browser(listener) => CanonicalValue::Object(BTreeMap::from([
            (
                "listener".to_owned(),
                CanonicalValue::String(listener.as_str().to_owned()),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("browser".to_owned()),
            ),
        ])),
    }
}

fn event_cycle_value(policy: EventCyclePolicy) -> CanonicalValue {
    match policy {
        EventCyclePolicy::ForbidRepeatedIsland => {
            CanonicalValue::String("forbid_repeated_island".to_owned())
        }
        EventCyclePolicy::MaximumHops(maximum_hops) => CanonicalValue::Object(BTreeMap::from([
            (
                "maximum_hops".to_owned(),
                CanonicalValue::String(maximum_hops.to_string()),
            ),
            (
                "type".to_owned(),
                CanonicalValue::String("maximum_hops".to_owned()),
            ),
        ])),
    }
}

fn reconnect_value(policy: ReconnectPolicy) -> CanonicalValue {
    match policy {
        ReconnectPolicy::RefreshOnReconnect => {
            CanonicalValue::String("refresh_on_reconnect".to_owned())
        }
        ReconnectPolicy::ResumeOrRefresh { maximum_attempts } => {
            CanonicalValue::Object(BTreeMap::from([
                (
                    "maximum_attempts".to_owned(),
                    CanonicalValue::String(maximum_attempts.to_string()),
                ),
                (
                    "type".to_owned(),
                    CanonicalValue::String("resume_or_refresh".to_owned()),
                ),
            ]))
        }
    }
}

const fn payload_schema_name(schema: BrowserPayloadSchema) -> &'static str {
    match schema {
        BrowserPayloadSchema::Json => "json",
        BrowserPayloadSchema::Null => "null",
        BrowserPayloadSchema::Boolean => "boolean",
        BrowserPayloadSchema::I64 => "i64",
        BrowserPayloadSchema::U64 => "u64",
        BrowserPayloadSchema::F64 => "f64",
        BrowserPayloadSchema::String => "string",
    }
}

const fn event_source_name(source: EventSource) -> &'static str {
    match source {
        EventSource::Component => "component",
        EventSource::Stream => "stream",
    }
}

const fn event_order_name(order: EventOrder) -> &'static str {
    match order {
        EventOrder::PerSourceSequence => "per_source_sequence",
    }
}

const fn subscription_mode_name(mode: SubscriptionMode) -> &'static str {
    match mode {
        SubscriptionMode::ServerSentEvents => "server_sent_events",
        SubscriptionMode::WebSocket => "web_socket",
    }
}

fn browser_operation_value(name: &str, version: u16) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("name".to_owned(), CanonicalValue::String(name.to_owned())),
        (
            "version".to_owned(),
            CanonicalValue::String(version.to_string()),
        ),
    ]))
}

const fn category_name(category: FieldCategory) -> &'static str {
    match category {
        FieldCategory::State => "state",
        FieldCategory::Public => "public",
        FieldCategory::Model => "model",
        FieldCategory::Locked => "locked",
        FieldCategory::ServerOnly => "server_only",
        FieldCategory::Session => "session",
        FieldCategory::Computed => "computed",
        FieldCategory::Transient => "transient",
        FieldCategory::Secret => "secret",
    }
}

const fn codec_name(codec: StateCodec) -> &'static str {
    match codec {
        StateCodec::Json => "json",
        StateCodec::I64Decimal => "i64_decimal",
        StateCodec::U64Decimal => "u64_decimal",
        StateCodec::BytesBase64Url => "bytes_base64url",
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use crate::async_updates::{
        BoundedTargets, BrowserPayloadSchema, EventCyclePolicy, EventOrder, EventSource,
        EventTarget,
    };

    use super::*;
    use crate::metadata::EventPayloadMetadata;

    struct DigestPayloadAlpha;

    impl EventPayloadMetadata for DigestPayloadAlpha {
        const NAME: &'static str = "digest_payload";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "digest_payload_alpha";
    }

    struct DigestPayloadBeta;

    impl EventPayloadMetadata for DigestPayloadBeta {
        const NAME: &'static str = "digest_payload";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "digest_payload_beta";
    }

    struct DigestNameChanged;

    impl EventPayloadMetadata for DigestNameChanged {
        const NAME: &'static str = "digest_payload_other";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "digest_payload_alpha";
    }

    struct DigestVersionChanged;

    impl EventPayloadMetadata for DigestVersionChanged {
        const NAME: &'static str = "digest_payload";
        const VERSION: u16 = 2;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "digest_payload_alpha";
    }

    struct DigestSchemaChanged;

    impl EventPayloadMetadata for DigestSchemaChanged {
        const NAME: &'static str = "digest_payload";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::String;
        const PAYLOAD_CONTRACT: &'static str = "digest_payload_alpha";
    }

    fn metadata<T: EventPayloadMetadata + 'static>() -> EventMetadata {
        metadata_with::<T>(
            EventSource::Component,
            EventTarget::SelfIsland,
            EventCyclePolicy::ForbidRepeatedIsland,
            8,
        )
    }

    fn metadata_with<T: EventPayloadMetadata + 'static>(
        source: EventSource,
        target: EventTarget,
        cycle: EventCyclePolicy,
        maximum_fanout: u16,
    ) -> EventMetadata {
        EventMetadata::from_payload_with_contract::<T>(
            source,
            BoundedTargets::new(vec![target]).expect("event target"),
            EventOrder::PerSourceSequence,
            cycle,
            maximum_fanout,
        )
        .expect("event metadata")
    }

    fn canonical_event_digest(value: &CanonicalValue) -> Vec<u8> {
        let limits = InputLimits::new(32 * 1_024, 8, 256, 256).expect("test limits");
        Sha256::digest(to_canonical_bytes(value, &limits).expect("canonical event value")).to_vec()
    }

    fn assert_only_event_field_changed(
        baseline: &CanonicalValue,
        variant: &CanonicalValue,
        expected_field: &str,
    ) {
        let CanonicalValue::Object(baseline_fields) = baseline else {
            panic!("baseline event contract must be a canonical object");
        };
        let CanonicalValue::Object(variant_fields) = variant else {
            panic!("variant event contract must be a canonical object");
        };
        assert_eq!(
            baseline_fields.keys().collect::<Vec<_>>(),
            variant_fields.keys().collect::<Vec<_>>()
        );
        let changed = baseline_fields
            .iter()
            .filter_map(|(name, value)| (variant_fields.get(name) != Some(value)).then_some(name))
            .map(String::as_str)
            .collect::<Vec<_>>();

        assert_eq!(changed, vec![expected_field]);
        assert_ne!(
            canonical_event_digest(baseline),
            canonical_event_digest(variant),
            "{expected_field} must be digest-significant"
        );
    }

    #[test]
    fn payload_contract_identity_is_independently_digest_significant() {
        let alpha = event_value(&metadata::<DigestPayloadAlpha>());
        let beta = event_value(&metadata::<DigestPayloadBeta>());

        assert_ne!(
            DigestPayloadAlpha::PAYLOAD_CONTRACT,
            DigestPayloadBeta::PAYLOAD_CONTRACT
        );
        assert_ne!(
            canonical_event_digest(&alpha),
            canonical_event_digest(&beta)
        );
    }

    #[test]
    fn every_event_field_is_independently_digest_significant() {
        let baseline = event_value(&metadata::<DigestPayloadAlpha>());
        let CanonicalValue::Object(fields) = &baseline else {
            panic!("event contract must be a canonical object");
        };
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "cycle",
                "maximum_fanout",
                "name",
                "order",
                "payload_contract",
                "schema",
                "source",
                "targets",
                "version",
            ]
        );
        assert_eq!(
            fields.get("order"),
            Some(&CanonicalValue::String("per_source_sequence".to_owned()))
        );

        let variants = [
            ("name", event_value(&metadata::<DigestNameChanged>())),
            ("version", event_value(&metadata::<DigestVersionChanged>())),
            (
                "payload_contract",
                event_value(&metadata::<DigestPayloadBeta>()),
            ),
            ("schema", event_value(&metadata::<DigestSchemaChanged>())),
            (
                "source",
                event_value(&metadata_with::<DigestPayloadAlpha>(
                    EventSource::Stream,
                    EventTarget::SelfIsland,
                    EventCyclePolicy::ForbidRepeatedIsland,
                    8,
                )),
            ),
            (
                "targets",
                event_value(&metadata_with::<DigestPayloadAlpha>(
                    EventSource::Component,
                    EventTarget::Document,
                    EventCyclePolicy::ForbidRepeatedIsland,
                    8,
                )),
            ),
            (
                "cycle",
                event_value(&metadata_with::<DigestPayloadAlpha>(
                    EventSource::Component,
                    EventTarget::SelfIsland,
                    EventCyclePolicy::MaximumHops(NonZeroU8::new(3).expect("nonzero hops")),
                    8,
                )),
            ),
            (
                "maximum_fanout",
                event_value(&metadata_with::<DigestPayloadAlpha>(
                    EventSource::Component,
                    EventTarget::SelfIsland,
                    EventCyclePolicy::ForbidRepeatedIsland,
                    9,
                )),
            ),
        ];
        for (field, variant) in variants {
            assert_only_event_field_changed(&baseline, &variant, field);
        }
    }
}
