//! Registered state schemas, bounded dehydration, and lossless tagged values.

use std::collections::BTreeMap;
use std::io::{self, Write};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{SnapshotError, SnapshotErrorKind};
use crate::canonical::{CanonicalValue, parse_canonical_value};
use crate::limits::InputLimits;

const TAG_FIELD: &str = "$live";
const VALUE_FIELD: &str = "value";

/// Snapshot eligibility category assigned by generated component metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldCategory {
    /// Ordinary instance-only state, which is the authoring default.
    State,
    /// Public inspectable state eligible for seed and instance snapshots.
    Public,
    /// Browser-proposable state eligible only for instanced snapshots.
    Model,
    /// Signed server-issued state eligible only for instanced snapshots.
    Locked,
    /// Server-only state that is never dehydrated.
    ServerOnly,
    /// Host-session-backed state that is never dehydrated.
    Session,
    /// Recomputed state that is never dehydrated.
    Computed,
    /// Request-only model state that is never dehydrated.
    Transient,
    /// Secret state that is never dehydrated or rendered.
    Secret,
}

/// Value codec registered for one dehydrated state field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateCodec {
    /// Ordinary canonical JSON within the interoperable number profile.
    Json,
    /// Signed 64-bit integer encoded with the `i64` decimal-string tag.
    I64Decimal,
    /// Unsigned 64-bit integer encoded with the `u64` decimal-string tag.
    U64Decimal,
    /// Arbitrary bytes encoded with the `bytes` base64url tag.
    BytesBase64Url,
}

/// Whether validation targets a reusable public seed or scoped instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateExposure {
    /// Only explicitly public fields are eligible.
    PublicSeed,
    /// Public, model, and locked fields are eligible.
    Instanced,
}

/// Registered metadata for one state, memo, or mount field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldSpec {
    name: String,
    codec: StateCodec,
    category: FieldCategory,
    required: bool,
}

impl FieldSpec {
    /// Creates one field with a bounded ASCII path segment.
    pub fn new(
        name: &str,
        codec: StateCodec,
        category: FieldCategory,
        required: bool,
    ) -> Result<Self, SnapshotError> {
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
        if !valid {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
        }
        Ok(Self {
            name: name.to_owned(),
            codec,
            category,
            required,
        })
    }
}

/// Versioned exact field schema selected by trusted component metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateSchema {
    version: u16,
    fields: BTreeMap<String, FieldSpec>,
}

impl StateSchema {
    /// Creates a non-zero schema version with unique field names.
    pub fn new(version: u16, fields: Vec<FieldSpec>) -> Result<Self, SnapshotError> {
        if version == 0 {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
        }
        let mut indexed = BTreeMap::new();
        for field in fields {
            if indexed.insert(field.name.clone(), field).is_some() {
                return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
            }
        }
        Ok(Self {
            version,
            fields: indexed,
        })
    }

    /// Returns the independent component-state schema version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Validates exact fields, exposure categories, required values, and codecs.
    pub fn validate(
        &self,
        value: &CanonicalValue,
        exposure: StateExposure,
    ) -> Result<(), SnapshotError> {
        let CanonicalValue::Object(values) = value else {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateShape));
        };

        for (name, value) in values {
            let field = self
                .fields
                .get(name)
                .ok_or_else(|| SnapshotError::new(SnapshotErrorKind::UnknownStateField))?;
            if !category_allowed(field.category, exposure) {
                return Err(SnapshotError::new(SnapshotErrorKind::ForbiddenStateField));
            }
            validate_codec(value, field.codec)?;
        }

        if self
            .fields
            .values()
            .any(|field| field.required && !values.contains_key(&field.name))
        {
            return Err(SnapshotError::new(SnapshotErrorKind::MissingStateField));
        }
        Ok(())
    }
}

fn category_allowed(category: FieldCategory, exposure: StateExposure) -> bool {
    match exposure {
        StateExposure::PublicSeed => category == FieldCategory::Public,
        StateExposure::Instanced => matches!(
            category,
            FieldCategory::State
                | FieldCategory::Public
                | FieldCategory::Model
                | FieldCategory::Locked
        ),
    }
}

fn validate_codec(value: &CanonicalValue, codec: StateCodec) -> Result<(), SnapshotError> {
    match codec {
        StateCodec::Json => Ok(()),
        StateCodec::I64Decimal => decode_i64(value).map(|_| ()),
        StateCodec::U64Decimal => decode_u64(value).map(|_| ()),
        StateCodec::BytesBase64Url => decode_bytes(value, 1024 * 1024).map(|_| ()),
    }
}

/// State, lifecycle memo, and public mount schemas bound by one component contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotSchemaSet {
    state: StateSchema,
    memo: StateSchema,
    mount: StateSchema,
}

impl SnapshotSchemaSet {
    /// Groups independently versioned registered schemas.
    pub fn new(
        state: StateSchema,
        memo: StateSchema,
        mount: StateSchema,
    ) -> Result<Self, SnapshotError> {
        Ok(Self { state, memo, mount })
    }

    /// Returns the component state schema.
    #[must_use]
    pub const fn state(&self) -> &StateSchema {
        &self.state
    }

    /// Returns the lifecycle memo schema.
    #[must_use]
    pub const fn memo(&self) -> &StateSchema {
        &self.memo
    }

    /// Returns the public mount-parameter schema.
    #[must_use]
    pub const fn mount(&self) -> &StateSchema {
        &self.mount
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(1_024)),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("state_output_limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serializes trusted Rust state into the bounded registered canonical schema.
pub fn dehydrate<T: Serialize>(
    value: &T,
    schema: &StateSchema,
    exposure: StateExposure,
    limits: &InputLimits,
) -> Result<CanonicalValue, SnapshotError> {
    let mut writer = BoundedWriter::new(limits.max_bytes());
    if serde_json::to_writer(&mut writer, value).is_err() {
        let kind = if writer.exceeded {
            SnapshotErrorKind::InputTooLarge
        } else {
            SnapshotErrorKind::DehydrationFailed
        };
        return Err(SnapshotError::new(kind));
    }
    let canonical = parse_canonical_value(&writer.bytes, limits).map_err(map_canonical_state)?;
    schema.validate(&canonical, exposure)?;
    Ok(canonical)
}

pub(crate) fn hydrate<T: DeserializeOwned>(
    value: &CanonicalValue,
    schema: &StateSchema,
    exposure: StateExposure,
) -> Result<T, SnapshotError> {
    schema.validate(value, exposure)?;
    let serde_value = value
        .to_serde_value()
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::HydrationFailed))?;
    serde_json::from_value(serde_value)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::HydrationFailed))
}

fn map_canonical_state(error: crate::canonical::CanonicalError) -> SnapshotError {
    use crate::canonical::CanonicalErrorKind;

    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge => SnapshotErrorKind::InputTooLarge,
        CanonicalErrorKind::TooDeep => SnapshotErrorKind::InputTooDeep,
        CanonicalErrorKind::TooManyEntries => SnapshotErrorKind::TooManyEntries,
        _ => SnapshotErrorKind::DehydrationFailed,
    };
    SnapshotError::new(kind)
}

fn tagged(tag: &str, value: String) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (TAG_FIELD.to_owned(), CanonicalValue::String(tag.to_owned())),
        (VALUE_FIELD.to_owned(), CanonicalValue::String(value)),
    ]))
}

fn decode_tag<'value>(
    value: &'value CanonicalValue,
    expected_tag: &str,
) -> Result<&'value str, SnapshotError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    };
    if fields.len() != 2 {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    }
    let Some(CanonicalValue::String(tag)) = fields.get(TAG_FIELD) else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    };
    let Some(CanonicalValue::String(value)) = fields.get(VALUE_FIELD) else {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    };
    if tag != expected_tag {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    }
    Ok(value)
}

/// Encodes a signed 64-bit integer without JavaScript precision loss.
#[must_use]
pub fn encode_i64(value: i64) -> CanonicalValue {
    tagged("i64", value.to_string())
}

/// Decodes an exact signed 64-bit tagged value.
pub fn decode_i64(value: &CanonicalValue) -> Result<i64, SnapshotError> {
    let encoded = decode_tag(value, "i64")?;
    let digits = encoded.strip_prefix('-').unwrap_or(encoded);
    let canonical = encoded == "0"
        || (!digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    }
    encoded
        .parse()
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidStateCodec))
}

/// Encodes an unsigned 64-bit integer without JavaScript precision loss.
#[must_use]
pub fn encode_u64(value: u64) -> CanonicalValue {
    tagged("u64", value.to_string())
}

/// Decodes an exact unsigned 64-bit tagged value.
pub fn decode_u64(value: &CanonicalValue) -> Result<u64, SnapshotError> {
    let encoded = decode_tag(value, "u64")?;
    let canonical = encoded == "0"
        || (!encoded.is_empty()
            && !encoded.starts_with('0')
            && encoded.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    }
    encoded
        .parse()
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidStateCodec))
}

/// Encodes bounded bytes as an unpadded base64url tagged value.
pub fn encode_bytes(value: &[u8], max_bytes: usize) -> Result<CanonicalValue, SnapshotError> {
    if value.len() > max_bytes {
        return Err(SnapshotError::new(SnapshotErrorKind::InputTooLarge));
    }
    Ok(tagged("bytes", URL_SAFE_NO_PAD.encode(value)))
}

/// Decodes bounded canonical unpadded base64url bytes.
pub fn decode_bytes(value: &CanonicalValue, max_bytes: usize) -> Result<Vec<u8>, SnapshotError> {
    let encoded = decode_tag(value, "bytes")?;
    if encoded.contains('=') || encoded.len() > max_bytes.saturating_mul(8).div_ceil(6) {
        return Err(SnapshotError::new(SnapshotErrorKind::InputTooLarge));
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| SnapshotError::new(SnapshotErrorKind::InvalidStateCodec))?;
    if decoded.len() > max_bytes || URL_SAFE_NO_PAD.encode(&decoded) != encoded {
        return Err(SnapshotError::new(SnapshotErrorKind::InvalidStateCodec));
    }
    Ok(decoded)
}
