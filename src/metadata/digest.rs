//! Purpose-specific canonical component-contract digest.

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{ComponentName, ContentDigest, ViewName};
use crate::limits::InputLimits;
use crate::snapshot::state::{FieldCategory, StateCodec};

use super::{ActionMetadata, ContractVersions, FieldMetadata, MetadataError, MetadataErrorKind};

const CONTRACT_DIGEST_DOMAIN: &[u8] = b"suprnova-live/component-contract/v1";

pub(super) fn contract_digest(
    identity: &ComponentName,
    view: &ViewName,
    versions: ContractVersions,
    fields: &[FieldMetadata],
    actions: &[ActionMetadata],
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
            "fields".to_owned(),
            CanonicalValue::Array(fields.iter().map(field_value).collect()),
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
            "category".to_owned(),
            CanonicalValue::String(category_name(field.category()).to_owned()),
        ),
        (
            "codec".to_owned(),
            CanonicalValue::String(codec_name(field.codec()).to_owned()),
        ),
        (
            "name".to_owned(),
            CanonicalValue::String(field.name().as_str().to_owned()),
        ),
        (
            "required".to_owned(),
            CanonicalValue::Bool(field.required()),
        ),
    ]))
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

const fn category_name(category: FieldCategory) -> &'static str {
    match category {
        FieldCategory::Public => "public",
        FieldCategory::Model => "model",
        FieldCategory::Locked => "locked",
        FieldCategory::ServerOnly => "server_only",
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
