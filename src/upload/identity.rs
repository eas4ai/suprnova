use std::error::Error;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::{Uuid, Version};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::crypto::{SnapshotKeyRing, SnapshotPurpose, SnapshotSignature};
use crate::host::{HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint};
use crate::identity::{
    ComponentName, ContentDigest, KeyId, ModelField, ScopeFingerprint, UnixMillis,
};
use crate::limits::InputLimits;

const GRANT_VERSION: &str = "v1";
const GRANT_SCHEMA_VERSION: u16 = 1;
const UPLOAD_PROTOCOL_V1: u16 = 1;
const MAX_GRANT_BYTES: usize = 4_096;
const MAX_CLAIMS_BYTES: usize = 2_048;
const MAX_ENCODED_CLAIMS_BYTES: usize = 2_731;
const GRANT_CLAIM_KEYS: [&str; 10] = [
    "component",
    "expires_at",
    "field",
    "handle",
    "principal",
    "protocol",
    "scope",
    "session",
    "tenant",
    "v",
];

/// Closed reason for rejecting upload identity or transfer authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadErrorKind {
    /// The upload handle was malformed, noncanonical, nil, or not UUIDv4/v7.
    InvalidHandle,
    /// The bearer grant token was malformed or exceeded its fixed bound.
    InvalidGrantEncoding,
    /// Grant integrity, canonical claims, key selection, or claim types failed.
    InvalidGrant,
    /// The grant is not current at the verification instant.
    GrantExpired,
    /// A valid grant was presented outside its bound upload or host scope.
    ScopeMismatch,
    /// The requested upload protocol is not implemented.
    UnsupportedProtocol,
    /// Encoded upload control input exceeded its byte budget.
    InputTooLarge,
    /// An upload control object repeated a JSON field.
    DuplicateField,
    /// The upload protocol operation name is not implemented.
    UnsupportedOperation,
    /// An upload control object contained a field outside its closed schema.
    UnknownField,
    /// A required upload control field was absent.
    MissingField,
    /// An upload control field had the wrong type, grammar, range, or value.
    InvalidField,
    /// The expected upload revision or retry identity conflicts with current state.
    UploadConflict,
    /// The requested state transition is not legal from the current state.
    InvalidTransition,
    /// The upload revision cannot advance beyond its integer bound.
    RevisionExhausted,
    /// The bounded retained idempotency outcome history is full.
    IdempotencyHistoryFull,
    /// The trusted request authority expired before upload admission.
    RequestAuthorityExpired,
    /// No current upload-authorization provider was available.
    AuthorizationUnavailable,
    /// Current principal or resource policy denied the upload operation.
    AuthorizationDenied,
    /// The upload authority ledger could not complete the requested operation.
    LedgerUnavailable,
    /// The bounded creation-rate window has no remaining capacity.
    CreationRateExceeded,
    /// The bounded pending-upload scope has no remaining capacity.
    PendingLimitExceeded,
    /// The bounded per-field upload count has no remaining capacity.
    FileCountExceeded,
    /// Temporary upload authority expired independently of its transfer grant.
    UploadExpired,
    /// The owning upload service lifecycle has retired.
    ServiceRetired,
    /// Server-side quarantine identity generation was unavailable.
    RandomUnavailable,
    /// The host quarantine provider could not complete bounded I/O.
    ProviderUnavailable,
    /// A generated quarantine object already existed.
    StorageConflict,
    /// The streaming request body ended with a transport failure.
    BodyInterrupted,
    /// Declared chunk or whole-file integrity did not match authoritative bytes.
    ChecksumMismatch,
    /// The complete authoritative byte range was not present.
    IncompleteTransfer,
    /// A recognized media header was truncated, malformed, or exceeded its parser cap.
    MediaHeaderUnproved,
    /// Accepted validation evidence was absent or did not match current authority.
    ValidationEvidenceUnavailable,
    /// A trusted durable-storage finalizer violated or could not complete its contract.
    FinalizationFailed,
    /// Cleanup of partially prepared durable work could not be confirmed.
    CompensationFailed,
    /// Durable work may exist but its lifecycle outcome still requires reconciliation.
    ReconciliationRequired,
    /// Per-upload or provider-wide cancellation stopped the operation.
    TransferCanceled,
    /// A bounded descriptor, chunk, queue, or memory permit was unavailable.
    ResourceExhausted,
}

impl UploadErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidHandle => "invalid_upload_handle",
            Self::InvalidGrantEncoding => "invalid_transfer_grant_encoding",
            Self::InvalidGrant => "invalid_transfer_grant",
            Self::GrantExpired => "transfer_grant_expired",
            Self::ScopeMismatch => "transfer_grant_scope_mismatch",
            Self::UnsupportedProtocol => "unsupported_protocol",
            Self::InputTooLarge => "input_too_large",
            Self::DuplicateField => "duplicate_field",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::UnknownField => "unknown_field",
            Self::MissingField => "missing_field",
            Self::InvalidField => "invalid_field",
            Self::UploadConflict => "upload_conflict",
            Self::InvalidTransition => "invalid_upload_transition",
            Self::RevisionExhausted => "upload_revision_exhausted",
            Self::IdempotencyHistoryFull => "upload_idempotency_history_full",
            Self::RequestAuthorityExpired => "upload_request_authority_expired",
            Self::AuthorizationUnavailable => "upload_authorization_unavailable",
            Self::AuthorizationDenied => "upload_authorization_denied",
            Self::LedgerUnavailable => "upload_ledger_unavailable",
            Self::CreationRateExceeded => "upload_creation_rate_exceeded",
            Self::PendingLimitExceeded => "upload_pending_limit_exceeded",
            Self::FileCountExceeded => "upload_file_count_exceeded",
            Self::UploadExpired => "upload_expired",
            Self::ServiceRetired => "upload_service_retired",
            Self::RandomUnavailable => "upload_random_unavailable",
            Self::ProviderUnavailable => "upload_provider_unavailable",
            Self::StorageConflict => "upload_storage_conflict",
            Self::BodyInterrupted => "upload_body_interrupted",
            Self::ChecksumMismatch => "upload_checksum_mismatch",
            Self::IncompleteTransfer => "upload_incomplete_transfer",
            Self::MediaHeaderUnproved => "upload_media_header_unproved",
            Self::ValidationEvidenceUnavailable => "upload_validation_evidence_unavailable",
            Self::FinalizationFailed => "upload_finalization_failed",
            Self::CompensationFailed => "upload_compensation_failed",
            Self::ReconciliationRequired => "upload_reconciliation_required",
            Self::TransferCanceled => "upload_transfer_canceled",
            Self::ResourceExhausted => "upload_resource_exhausted",
        }
    }
}

/// Redacted upload identity or transfer-authority failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UploadError {
    kind: UploadErrorKind,
}

impl UploadError {
    /// Constructs one closed redacted failure for a host implementation.
    #[must_use]
    pub const fn new(kind: UploadErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> UploadErrorKind {
        self.kind
    }
}

impl fmt::Display for UploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for UploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for UploadError {}

/// Server-generated, non-authoritative identity for one temporary upload.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct UploadHandle(Uuid);

impl UploadHandle {
    /// Parses a canonical lowercase hyphenated, non-nil UUIDv4 or UUIDv7 handle.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let uuid =
            Uuid::parse_str(value).map_err(|_| UploadError::new(UploadErrorKind::InvalidHandle))?;
        let canonical = uuid.hyphenated().to_string();
        let server_generated = matches!(
            uuid.get_version(),
            Some(Version::Random | Version::SortRand)
        );
        if canonical != value || uuid.is_nil() || !server_generated {
            return Err(UploadError::new(UploadErrorKind::InvalidHandle));
        }
        Ok(Self(uuid))
    }
}

impl fmt::Debug for UploadHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadHandle>")
    }
}

impl fmt::Display for UploadHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.hyphenated())
    }
}

impl Serialize for UploadHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UploadHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Secret bearer capability authorizing bounded transfer operations.
///
/// This type deliberately implements neither `Display` nor `Serialize`. The
/// explicit bearer accessor is the only way a host adapter can place the secret
/// in an authorization header.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct TransferGrant(Zeroizing<Vec<u8>>);

impl TransferGrant {
    /// Parses the bounded structural envelope without verifying its authority.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        parse_grant_envelope(value)?;
        Ok(Self(Zeroizing::new(value.as_bytes().to_vec())))
    }

    /// Exposes the bearer token for an authorization header.
    ///
    /// Callers must not place this value in URLs, HTML, snapshots, logs,
    /// diagnostics, traces, history, or model/action payloads.
    #[must_use]
    pub fn expose_bearer(&self) -> &str {
        std::str::from_utf8(self.0.as_slice()).map_or("", |value| value)
    }
}

impl fmt::Debug for TransferGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransferGrant:redacted>")
    }
}

/// Complete authority facts to which one transfer grant is bound.
#[derive(Clone, Eq, PartialEq)]
pub struct TransferGrantScope {
    handle: UploadHandle,
    component: ComponentName,
    field: ModelField,
    host_scope: HostScopeFacts,
    upload_protocol: u16,
}

impl TransferGrantScope {
    /// Groups the upload identity, component field, host scope, and protocol.
    #[must_use]
    pub const fn new(
        handle: UploadHandle,
        component: ComponentName,
        field: ModelField,
        host_scope: HostScopeFacts,
        upload_protocol: u16,
    ) -> Self {
        Self {
            handle,
            component,
            field,
            host_scope,
            upload_protocol,
        }
    }

    /// Returns the non-authoritative upload handle.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the registered component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the declared upload model field.
    #[must_use]
    pub const fn field(&self) -> &ModelField {
        &self.field
    }

    /// Returns the normalized current host scope.
    #[must_use]
    pub const fn host_scope(&self) -> &HostScopeFacts {
        &self.host_scope
    }

    /// Returns the independently versioned upload protocol.
    #[must_use]
    pub const fn upload_protocol(&self) -> u16 {
        self.upload_protocol
    }
}

impl fmt::Debug for TransferGrantScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransferGrantScope:redacted>")
    }
}

/// Issuance request for one expiring transfer grant.
#[derive(Clone, Eq, PartialEq)]
pub struct TransferGrantRequest {
    authority: TransferGrantScope,
    expires_at: UnixMillis,
}

impl TransferGrantRequest {
    /// Binds an upload authority scope to an exclusive expiry instant.
    #[must_use]
    pub const fn new(authority: TransferGrantScope, expires_at: UnixMillis) -> Self {
        Self {
            authority,
            expires_at,
        }
    }
}

impl fmt::Debug for TransferGrantRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransferGrantRequest:redacted>")
    }
}

/// Newly issued non-authoritative handle and separate secret transfer grant.
pub struct IssuedTransferGrant {
    handle: UploadHandle,
    grant: TransferGrant,
}

impl IssuedTransferGrant {
    /// Returns the non-authoritative upload handle.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the separate bearer capability.
    #[must_use]
    pub const fn grant(&self) -> &TransferGrant {
        &self.grant
    }
}

impl fmt::Debug for IssuedTransferGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<IssuedTransferGrant:redacted>")
    }
}

/// Verified transfer capability whose claims matched the current request scope.
#[derive(Clone, Eq, PartialEq)]
pub struct VerifiedTransferGrant {
    authority: TransferGrantScope,
    expires_at: UnixMillis,
}

impl VerifiedTransferGrant {
    /// Returns the verified non-authoritative upload handle.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.authority.handle()
    }

    /// Returns the verified component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.authority.component()
    }

    /// Returns the verified upload model field.
    #[must_use]
    pub const fn field(&self) -> &ModelField {
        self.authority.field()
    }

    /// Returns the verified current host scope.
    #[must_use]
    pub const fn scope(&self) -> &HostScopeFacts {
        self.authority.host_scope()
    }

    /// Returns the exclusive grant expiry instant.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Returns the verified upload protocol.
    #[must_use]
    pub const fn upload_protocol(&self) -> u16 {
        self.authority.upload_protocol()
    }
}

impl fmt::Debug for VerifiedTransferGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifiedTransferGrant:redacted>")
    }
}

/// Purpose-separated issuer and verifier backed by the engine key ring.
pub struct TransferGrantCodec {
    keys: SnapshotKeyRing,
}

impl TransferGrantCodec {
    /// Creates a codec using the configured active and overlapping keys.
    #[must_use]
    pub const fn new(keys: SnapshotKeyRing) -> Self {
        Self { keys }
    }

    /// Issues one canonical expiring bearer grant.
    pub fn issue(
        &self,
        request: TransferGrantRequest,
        now: UnixMillis,
    ) -> Result<IssuedTransferGrant, UploadError> {
        if request.authority.upload_protocol != UPLOAD_PROTOCOL_V1 {
            return Err(UploadError::new(UploadErrorKind::UnsupportedProtocol));
        }
        if request.expires_at <= now {
            return Err(UploadError::new(UploadErrorKind::GrantExpired));
        }

        let claims = GrantClaims::from_request(&request);
        let body = encode_claims(&claims)?;
        let signed = self
            .keys
            .sign(SnapshotPurpose::UploadGrantV1, &body, now)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
        let token = format!(
            "{GRANT_VERSION}.{}.{}.{}",
            signed.key_id().as_str(),
            URL_SAFE_NO_PAD.encode(&body),
            signed.signature().to_base64url()
        );
        let grant = TransferGrant::parse(&token)?;

        Ok(IssuedTransferGrant {
            handle: request.authority.handle,
            grant,
        })
    }

    /// Verifies integrity, canonical claims, expiry, and exact request scope.
    pub fn verify(
        &self,
        grant: &TransferGrant,
        expected: &TransferGrantScope,
        now: UnixMillis,
    ) -> Result<VerifiedTransferGrant, UploadError> {
        let envelope = parse_grant_envelope(grant.expose_bearer())?;
        let body = decode_claim_body(envelope.body)?;
        self.keys
            .verify(
                &envelope.key_id,
                SnapshotPurpose::UploadGrantV1,
                &body,
                &envelope.signature,
                now,
            )
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;

        let claims = decode_claims(&body)?;
        let authority = claims.authority()?;
        let expires_at = UnixMillis::parse(&claims.expires_at)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
        if expires_at <= now {
            return Err(UploadError::new(UploadErrorKind::GrantExpired));
        }
        if authority != *expected {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }

        Ok(VerifiedTransferGrant {
            authority,
            expires_at,
        })
    }
}

impl fmt::Debug for TransferGrantCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransferGrantCodec:redacted>")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GrantClaims {
    component: String,
    expires_at: String,
    field: String,
    handle: String,
    principal: Option<String>,
    protocol: u16,
    scope: String,
    session: Option<String>,
    tenant: Option<String>,
    v: u16,
}

impl GrantClaims {
    fn from_request(request: &TransferGrantRequest) -> Self {
        let authority = &request.authority;
        let host = &authority.host_scope;
        Self {
            component: authority.component.as_str().to_owned(),
            expires_at: request.expires_at.get().to_string(),
            field: authority.field.as_str().to_owned(),
            handle: authority.handle.to_string(),
            principal: host.principal().map(|value| value.digest().to_base64url()),
            protocol: authority.upload_protocol,
            scope: host.scope().to_base64url(),
            session: host.session().map(|value| value.digest().to_base64url()),
            tenant: host.tenant().map(|value| value.digest().to_base64url()),
            v: GRANT_SCHEMA_VERSION,
        }
    }

    fn authority(&self) -> Result<TransferGrantScope, UploadError> {
        if self.v != GRANT_SCHEMA_VERSION || self.protocol != UPLOAD_PROTOCOL_V1 {
            return Err(UploadError::new(UploadErrorKind::InvalidGrant));
        }
        let handle = UploadHandle::parse(&self.handle)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
        let component = ComponentName::parse(&self.component)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
        let field = ModelField::parse(&self.field)
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
        let host_scope = HostScopeFacts::new(
            parse_scope_fingerprint(&self.scope)?,
            self.session
                .as_deref()
                .map(parse_session_fingerprint)
                .transpose()?,
            self.principal
                .as_deref()
                .map(parse_principal_fingerprint)
                .transpose()?,
            self.tenant
                .as_deref()
                .map(parse_tenant_fingerprint)
                .transpose()?,
        );
        Ok(TransferGrantScope::new(
            handle,
            component,
            field,
            host_scope,
            self.protocol,
        ))
    }
}

struct GrantEnvelope<'a> {
    key_id: KeyId,
    body: &'a str,
    signature: SnapshotSignature,
}

fn parse_grant_envelope(value: &str) -> Result<GrantEnvelope<'_>, UploadError> {
    if value.is_empty() || value.len() > MAX_GRANT_BYTES || !value.is_ascii() {
        return Err(UploadError::new(UploadErrorKind::InvalidGrantEncoding));
    }
    let mut parts = value.split('.');
    let version = parts.next();
    let key_id = parts.next();
    let body = parts.next();
    let signature = parts.next();
    if version != Some(GRANT_VERSION)
        || parts.next().is_some()
        || key_id.is_none_or(str::is_empty)
        || body.is_none_or(str::is_empty)
        || signature.is_none_or(str::is_empty)
    {
        return Err(UploadError::new(UploadErrorKind::InvalidGrantEncoding));
    }
    let key_id = KeyId::parse(key_id.unwrap_or_default())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrantEncoding))?;
    let body = body.unwrap_or_default();
    if body.len() > MAX_ENCODED_CLAIMS_BYTES
        || body.contains('=')
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(UploadError::new(UploadErrorKind::InvalidGrantEncoding));
    }
    let signature = SnapshotSignature::parse(signature.unwrap_or_default())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrantEncoding))?;
    Ok(GrantEnvelope {
        key_id,
        body,
        signature,
    })
}

fn grant_limits() -> Result<InputLimits, UploadError> {
    InputLimits::new(MAX_CLAIMS_BYTES, 4, 16, 256)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn encode_claims(claims: &GrantClaims) -> Result<Vec<u8>, UploadError> {
    let serde_value = serde_json::to_value(claims)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
    let canonical = CanonicalValue::from_serde_value(serde_value)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
    to_canonical_bytes(&canonical, &grant_limits()?)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn decode_claim_body(encoded: &str) -> Result<Vec<u8>, UploadError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrantEncoding))?;
    if decoded.len() > MAX_CLAIMS_BYTES || URL_SAFE_NO_PAD.encode(&decoded) != encoded {
        return Err(UploadError::new(UploadErrorKind::InvalidGrantEncoding));
    }
    Ok(decoded)
}

fn decode_claims(body: &[u8]) -> Result<GrantClaims, UploadError> {
    let canonical = parse_canonical_value(body, &grant_limits()?)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
    let CanonicalValue::Object(fields) = &canonical else {
        return Err(UploadError::new(UploadErrorKind::InvalidGrant));
    };
    if fields.len() != GRANT_CLAIM_KEYS.len()
        || !GRANT_CLAIM_KEYS.iter().all(|key| fields.contains_key(*key))
    {
        return Err(UploadError::new(UploadErrorKind::InvalidGrant));
    }
    let encoded = to_canonical_bytes(&canonical, &grant_limits()?)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
    if encoded != body {
        return Err(UploadError::new(UploadErrorKind::InvalidGrant));
    }
    let serde_value = canonical
        .to_serde_value()
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))?;
    serde_json::from_value(serde_value).map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn parse_digest(value: &str) -> Result<ContentDigest, UploadError> {
    ContentDigest::parse(value).map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn parse_scope_fingerprint(value: &str) -> Result<ScopeFingerprint, UploadError> {
    let digest = parse_digest(value)?;
    ScopeFingerprint::from_bytes(digest.as_bytes())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn parse_session_fingerprint(value: &str) -> Result<SessionFingerprint, UploadError> {
    let digest = parse_digest(value)?;
    SessionFingerprint::from_bytes(digest.as_bytes())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn parse_principal_fingerprint(value: &str) -> Result<PrincipalFingerprint, UploadError> {
    let digest = parse_digest(value)?;
    PrincipalFingerprint::from_bytes(digest.as_bytes())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}

fn parse_tenant_fingerprint(value: &str) -> Result<TenantFingerprint, UploadError> {
    let digest = parse_digest(value)?;
    TenantFingerprint::from_bytes(digest.as_bytes())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidGrant))
}
