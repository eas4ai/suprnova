//! Purpose-specific canonical component-contract digest.

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{ComponentName, ContentDigest, ViewName};
use crate::limits::InputLimits;
use crate::snapshot::state::{FieldCategory, StateCodec};
use crate::state::{BindingTiming, ModelCodec, UrlBindingMode};

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
    ]))
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
            "name".to_owned(),
            CanonicalValue::String(action.name().as_str().to_owned()),
        ),
        (
            "version".to_owned(),
            CanonicalValue::String(action.version().to_string()),
        ),
    ]))
}

fn event_value(event: &EventMetadata) -> CanonicalValue {
    browser_operation_value(event.name().as_str(), event.version())
}

fn effect_value(effect: &EffectMetadata) -> CanonicalValue {
    browser_operation_value(effect.name().as_str(), effect.version())
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
