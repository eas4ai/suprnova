use std::collections::BTreeMap;
use std::fmt;

use crate::canonical::{CanonicalErrorKind, CanonicalValue, parse_canonical_value};
use crate::identity::ModelField;
use crate::limits::InputLimits;

use super::{UploadError, UploadErrorKind, UploadHandle};

/// Independently versioned upload protocol majors understood by the engine.
pub const SUPPORTED_UPLOAD_PROTOCOL_VERSIONS: &[u16] = &[1];

const UPLOAD_PROTOCOL_V1: u16 = 1;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
const SHA256_HEX_BYTES: usize = 64;
const MAX_SAFE_JSON_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Monotonic revision of one temporary upload record.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UploadRevision(u64);

impl UploadRevision {
    /// Creates a revision from trusted state.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the first persisted upload revision.
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    /// Parses the canonical unsigned-decimal wire representation.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let canonical = value == "0"
            || (!value.is_empty()
                && !value.starts_with('0')
                && value.bytes().all(|byte| byte.is_ascii_digit()));
        if !canonical {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))
    }

    /// Returns the underlying revision number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn checked_next(self) -> Result<Self, UploadError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| UploadError::new(UploadErrorKind::RevisionExhausted))
    }
}

/// Bounded retry identity scoped to one upload operation.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UploadIdempotencyKey(String);

impl UploadIdempotencyKey {
    /// Parses the bounded ASCII retry-key grammar.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_IDEMPOTENCY_KEY_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
            });
        if !valid {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated retry identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for UploadIdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadIdempotencyKey>")
    }
}

/// Validated lowercase hexadecimal SHA-256 checksum.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct UploadChecksum(String);

impl UploadChecksum {
    /// Parses exactly 32 bytes of lowercase hexadecimal digest text.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let valid = value.len() == SHA256_HEX_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
        if !valid {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated lowercase hexadecimal checksum.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for UploadChecksum {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadChecksum>")
    }
}

/// Creates one temporary upload for a declared model field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateUpload {
    expected_revision: UploadRevision,
    field: ModelField,
    idempotency_key: UploadIdempotencyKey,
}

impl CreateUpload {
    /// Returns the expected absent-record revision, normally zero.
    #[must_use]
    pub const fn expected_revision(&self) -> UploadRevision {
        self.expected_revision
    }

    /// Returns the declared upload model field.
    #[must_use]
    pub const fn field(&self) -> &ModelField {
        &self.field
    }

    /// Returns the retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }
}

/// Transfers one bounded indexed chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PutChunk {
    handle: UploadHandle,
    expected_revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
    chunk_index: u32,
    size: u64,
    checksum: UploadChecksum,
}

impl PutChunk {
    /// Returns the target temporary upload.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the expected current revision.
    #[must_use]
    pub const fn expected_revision(&self) -> UploadRevision {
        self.expected_revision
    }

    /// Returns the retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the zero-based chunk index.
    #[must_use]
    pub const fn chunk_index(&self) -> u32 {
        self.chunk_index
    }

    /// Returns the declared non-zero chunk size.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns the declared chunk checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        &self.checksum
    }
}

/// Reads current upload status without mutating the record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusUpload {
    handle: UploadHandle,
}

impl StatusUpload {
    /// Returns the target temporary upload.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }
}

/// Declares transfer completion and the authoritative whole-file checksum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteUpload {
    handle: UploadHandle,
    expected_revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
    whole_checksum: UploadChecksum,
}

impl CompleteUpload {
    /// Returns the target temporary upload.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the expected current revision.
    #[must_use]
    pub const fn expected_revision(&self) -> UploadRevision {
        self.expected_revision
    }

    /// Returns the retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the declared whole-file checksum.
    #[must_use]
    pub const fn whole_checksum(&self) -> &UploadChecksum {
        &self.whole_checksum
    }
}

/// Cancels one pending temporary upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelUpload {
    handle: UploadHandle,
    expected_revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
}

impl CancelUpload {
    /// Returns the target temporary upload.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the expected current revision.
    #[must_use]
    pub const fn expected_revision(&self) -> UploadRevision {
        self.expected_revision
    }

    /// Returns the retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }
}

/// Requests a new grant through an authenticated application-owned route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReacquireUpload {
    handle: UploadHandle,
}

impl ReacquireUpload {
    /// Returns the non-authoritative handle to reauthorize.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }
}

/// Closed external upload protocol-v1 operation vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadOperation {
    /// Creates a temporary upload.
    Create(CreateUpload),
    /// Transfers one chunk.
    PutChunk(PutChunk),
    /// Reads current status.
    Status(StatusUpload),
    /// Completes byte transfer.
    Complete(CompleteUpload),
    /// Cancels pending work.
    Cancel(CancelUpload),
    /// Reauthorizes through an application-owned route.
    Reacquire(ReacquireUpload),
}

impl UploadOperation {
    /// Returns the stable wire operation name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::PutChunk(_) => "put_chunk",
            Self::Status(_) => "status",
            Self::Complete(_) => "complete",
            Self::Cancel(_) => "cancel",
            Self::Reacquire(_) => "reacquire",
        }
    }
}

/// Bounded decoder for the independent upload control protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadProtocolCodec {
    limits: InputLimits,
}

impl UploadProtocolCodec {
    /// Validates host-selected JSON bounds.
    pub fn new(
        max_bytes: usize,
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
    ) -> Result<Self, UploadError> {
        InputLimits::new(max_bytes, max_depth, max_entries, max_string_bytes)
            .map(|limits| Self { limits })
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))
    }

    /// Returns the locked protocol-v1 codec.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            limits: InputLimits::upload_protocol_v1(),
        }
    }

    /// Decodes one exact closed-schema upload command.
    pub fn decode(&self, input: &[u8]) -> Result<UploadOperation, UploadError> {
        let value = parse_canonical_value(input, &self.limits).map_err(map_canonical_error)?;
        let CanonicalValue::Object(mut fields) = value else {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        };
        let protocol = required_u16(&mut fields, "protocol_version")?;
        if protocol != UPLOAD_PROTOCOL_V1 {
            return Err(UploadError::new(UploadErrorKind::UnsupportedProtocol));
        }
        let operation = required_string(&mut fields, "operation")?;
        match operation.as_str() {
            "create" => decode_create(fields),
            "put_chunk" => decode_put_chunk(fields),
            "status" => decode_status(fields),
            "complete" => decode_complete(fields),
            "cancel" => decode_cancel(fields),
            "reacquire" => decode_reacquire(fields),
            _ => Err(UploadError::new(UploadErrorKind::UnsupportedOperation)),
        }
    }
}

fn decode_create(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(&fields, &["expected_revision", "field", "idempotency_key"])?;
    let expected_revision = required_revision(&mut fields)?;
    if expected_revision.get() != 0 {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    }
    let field = ModelField::parse(&required_string(&mut fields, "field")?)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
    let idempotency_key = required_idempotency(&mut fields)?;
    Ok(UploadOperation::Create(CreateUpload {
        expected_revision,
        field,
        idempotency_key,
    }))
}

fn decode_put_chunk(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(
        &fields,
        &[
            "checksum",
            "chunk_index",
            "expected_revision",
            "handle",
            "idempotency_key",
            "size",
        ],
    )?;
    let handle = required_handle(&mut fields)?;
    let expected_revision = required_revision(&mut fields)?;
    let idempotency_key = required_idempotency(&mut fields)?;
    let chunk_index = required_u32(&mut fields, "chunk_index")?;
    let size = required_u64(&mut fields, "size")?;
    if size == 0 {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    }
    let checksum = UploadChecksum::parse(&required_string(&mut fields, "checksum")?)?;
    Ok(UploadOperation::PutChunk(PutChunk {
        handle,
        expected_revision,
        idempotency_key,
        chunk_index,
        size,
        checksum,
    }))
}

fn decode_status(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(&fields, &["handle"])?;
    Ok(UploadOperation::Status(StatusUpload {
        handle: required_handle(&mut fields)?,
    }))
}

fn decode_complete(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(
        &fields,
        &[
            "expected_revision",
            "handle",
            "idempotency_key",
            "whole_checksum",
        ],
    )?;
    Ok(UploadOperation::Complete(CompleteUpload {
        handle: required_handle(&mut fields)?,
        expected_revision: required_revision(&mut fields)?,
        idempotency_key: required_idempotency(&mut fields)?,
        whole_checksum: UploadChecksum::parse(&required_string(&mut fields, "whole_checksum")?)?,
    }))
}

fn decode_cancel(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(&fields, &["expected_revision", "handle", "idempotency_key"])?;
    Ok(UploadOperation::Cancel(CancelUpload {
        handle: required_handle(&mut fields)?,
        expected_revision: required_revision(&mut fields)?,
        idempotency_key: required_idempotency(&mut fields)?,
    }))
}

fn decode_reacquire(
    mut fields: BTreeMap<String, CanonicalValue>,
) -> Result<UploadOperation, UploadError> {
    reject_unknown(&fields, &["handle"])?;
    Ok(UploadOperation::Reacquire(ReacquireUpload {
        handle: required_handle(&mut fields)?,
    }))
}

fn map_canonical_error(error: crate::canonical::CanonicalError) -> UploadError {
    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge => UploadErrorKind::InputTooLarge,
        CanonicalErrorKind::DuplicateKey => UploadErrorKind::DuplicateField,
        _ => UploadErrorKind::InvalidField,
    };
    UploadError::new(kind)
}

fn reject_unknown(
    fields: &BTreeMap<String, CanonicalValue>,
    allowed: &[&str],
) -> Result<(), UploadError> {
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(UploadError::new(UploadErrorKind::UnknownField));
    }
    Ok(())
}

fn required_value(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<CanonicalValue, UploadError> {
    fields
        .remove(name)
        .ok_or_else(|| UploadError::new(UploadErrorKind::MissingField))
}

fn required_string(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<String, UploadError> {
    let CanonicalValue::String(value) = required_value(fields, name)? else {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    };
    Ok(value)
}

fn required_number(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<f64, UploadError> {
    let CanonicalValue::Number(value) = required_value(fields, name)? else {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    };
    let value = value.get();
    if value < 0.0 || value.fract() != 0.0 || value > MAX_SAFE_JSON_INTEGER {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    }
    Ok(value)
}

fn required_u16(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u16, UploadError> {
    let value = required_number(fields, name)?;
    if value > f64::from(u16::MAX) {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    }
    Ok(value as u16)
}

fn required_u32(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u32, UploadError> {
    let value = required_number(fields, name)?;
    if value > f64::from(u32::MAX) {
        return Err(UploadError::new(UploadErrorKind::InvalidField));
    }
    Ok(value as u32)
}

fn required_u64(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u64, UploadError> {
    Ok(required_number(fields, name)? as u64)
}

fn required_revision(
    fields: &mut BTreeMap<String, CanonicalValue>,
) -> Result<UploadRevision, UploadError> {
    UploadRevision::parse(&required_string(fields, "expected_revision")?)
}

fn required_idempotency(
    fields: &mut BTreeMap<String, CanonicalValue>,
) -> Result<UploadIdempotencyKey, UploadError> {
    UploadIdempotencyKey::parse(&required_string(fields, "idempotency_key")?)
}

fn required_handle(
    fields: &mut BTreeMap<String, CanonicalValue>,
) -> Result<UploadHandle, UploadError> {
    UploadHandle::parse(&required_string(fields, "handle")?)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))
}
