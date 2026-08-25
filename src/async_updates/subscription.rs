//! Canonical signed subscription descriptions and separately secret credentials.

use std::error::Error;
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::crypto::{SnapshotKeyRing, SnapshotPurpose, SnapshotSignature};
use crate::identity::{BrowserOperationName, KeyId, UnixMillis};
use crate::limits::InputLimits;

use super::{BoundedEventNames, BoundedTopics, ReconnectPolicy, StreamName, TopicName};

/// Independently versioned asynchronous subscription protocol implemented here.
pub const ASYNC_SUBSCRIPTION_PROTOCOL_V1: u16 = 1;
/// Minimum hybrid fallback-poll interval.
pub const MIN_POLL_INTERVAL_MS: u64 = 1_000;
/// Maximum hybrid fallback-poll interval.
pub const MAX_POLL_INTERVAL_MS: u64 = 300_000;
/// Maximum jitter expressed in basis points (100 percent).
pub const MAX_POLL_JITTER_BASIS_POINTS: u16 = 10_000;
/// Maximum continuity-resume attempts before an authoritative refresh.
pub const MAX_RECONNECT_ATTEMPTS: u8 = 16;
/// Maximum lifetime of one issued subscription descriptor.
pub const MAX_SUBSCRIPTION_LIFETIME_MS: u64 = 300_000;

const DESCRIPTOR_VERSION: &str = "as1";
const DESCRIPTOR_SCHEMA_VERSION: u16 = 1;
const MAX_DESCRIPTOR_BYTES: usize = 16_384;
const MAX_ENCODED_CLAIMS_BYTES: usize = 12_000;
const MAX_CLAIMS_BYTES: usize = 8_192;
const MAX_AUTHORIZATION_MEMO_BYTES: usize = 512;
const MIN_TRANSPORT_CREDENTIAL_BYTES: usize = 16;
const MAX_TRANSPORT_CREDENTIAL_BYTES: usize = 1_024;
const CLAIM_KEYS: [&str; 11] = [
    "authorization_memo",
    "baseline",
    "capability",
    "events",
    "expires_at",
    "fallback_poll",
    "protocol",
    "reconnect",
    "stream",
    "topics",
    "v",
];

/// Closed reason for rejecting a subscription descriptor or authorization boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionErrorKind {
    /// A descriptor envelope, canonical body, key, or signature was invalid.
    InvalidDescriptor,
    /// The descriptor or requested lifetime reached its exclusive deadline.
    DescriptorExpired,
    /// The independently versioned protocol is unsupported.
    UnsupportedProtocol,
    /// The declared capability version is invalid.
    InvalidCapability,
    /// The authorization context memo is malformed or unbounded.
    InvalidAuthorizationMemo,
    /// The hybrid fallback polling contract exceeds a hard bound.
    InvalidPollFallback,
    /// The reconnect policy exceeds its hard attempt bound.
    InvalidReconnectPolicy,
    /// Verified claims do not match current principal, tenant, component, stream, or topics.
    ScopeMismatch,
    /// Current host authorization was unavailable.
    AuthorizationUnavailable,
    /// Current host policy denied the operation.
    AuthorizationDenied,
    /// The separate transport credential service was unavailable.
    CredentialUnavailable,
    /// The separate transport credential was invalid for this descriptor binding.
    InvalidCredential,
    /// A trusted request context was expired before subscription work.
    ContextExpired,
    /// The selected stream is absent from the registry-verified component contract.
    UnregisteredSubscription,
}

impl SubscriptionErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDescriptor => "invalid_subscription_descriptor",
            Self::DescriptorExpired => "subscription_descriptor_expired",
            Self::UnsupportedProtocol => "unsupported_subscription_protocol",
            Self::InvalidCapability => "invalid_subscription_capability",
            Self::InvalidAuthorizationMemo => "invalid_authorization_memo",
            Self::InvalidPollFallback => "invalid_poll_fallback",
            Self::InvalidReconnectPolicy => "invalid_reconnect_policy",
            Self::ScopeMismatch => "subscription_scope_mismatch",
            Self::AuthorizationUnavailable => "subscription_authorization_unavailable",
            Self::AuthorizationDenied => "subscription_authorization_denied",
            Self::CredentialUnavailable => "transport_credential_unavailable",
            Self::InvalidCredential => "invalid_transport_credential",
            Self::ContextExpired => "subscription_context_expired",
            Self::UnregisteredSubscription => "unregistered_subscription",
        }
    }
}

/// Redacted subscription rejection.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SubscriptionError {
    kind: SubscriptionErrorKind,
}

impl SubscriptionError {
    pub(crate) const fn new(kind: SubscriptionErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed safe rejection reason.
    #[must_use]
    pub const fn kind(self) -> SubscriptionErrorKind {
        self.kind
    }
}

impl fmt::Display for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for SubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for SubscriptionError {}

/// Nonzero independently negotiated subscription capability version.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CapabilityVersion(u16);

impl CapabilityVersion {
    /// Creates a nonzero capability version.
    pub fn new(value: u16) -> Result<Self, SubscriptionError> {
        if value == 0 {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidCapability,
            ));
        }
        Ok(Self(value))
    }

    /// Returns the negotiated version.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Stream epoch chosen by server continuity authority.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StreamEpoch(u64);

impl StreamEpoch {
    /// Creates an epoch value. Epoch interpretation belongs to the stream provider.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the epoch value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Monotonic sequence within one server-authoritative stream epoch.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StreamSequence(u64);

impl StreamSequence {
    /// Creates a sequence value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the sequence value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Authoritative server baseline for the first required stream event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamPosition {
    epoch: StreamEpoch,
    sequence: StreamSequence,
}

impl StreamPosition {
    /// Groups one epoch and sequence baseline without applying sequence-machine behavior.
    #[must_use]
    pub const fn new(epoch: StreamEpoch, sequence: StreamSequence) -> Self {
        Self { epoch, sequence }
    }

    /// Returns the stream epoch.
    #[must_use]
    pub const fn epoch(self) -> StreamEpoch {
        self.epoch
    }

    /// Returns the sequence within the epoch.
    #[must_use]
    pub const fn sequence(self) -> StreamSequence {
        self.sequence
    }
}

/// Bounded, non-secret memo binding descriptor issuance to host authorization context.
#[derive(Clone, Eq, PartialEq)]
pub struct AuthorizationMemo(String);

impl AuthorizationMemo {
    /// Parses a bounded printable non-secret authorization memo.
    pub fn parse(value: &str) -> Result<Self, SubscriptionError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_AUTHORIZATION_MEMO_BYTES
            && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e));
        if !valid {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidAuthorizationMemo,
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the bounded non-secret memo.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for AuthorizationMemo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizationMemo>")
    }
}

/// Whether hybrid fallback polling performs an initial refresh immediately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollInitialBehavior {
    /// Queue one registered fresh-render operation immediately.
    Immediate,
    /// Wait for the first bounded interval.
    AfterInterval,
}

/// Hidden-document behavior for hybrid fallback polling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollVisibilityPolicy {
    /// Pause fallback work while the document is hidden.
    PauseWhenHidden,
    /// Continue the bounded interval while hidden.
    ContinueWhenHidden,
}

/// Authoritative bounded default used when hybrid push continuity is uncertain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PollFallbackPolicy {
    interval_ms: u64,
    jitter_basis_points: u16,
    initial: PollInitialBehavior,
    visibility: PollVisibilityPolicy,
}

impl PollFallbackPolicy {
    /// Creates a fallback policy within fixed interval and jitter bounds.
    pub fn new(
        interval_ms: u64,
        jitter_basis_points: u16,
        initial: PollInitialBehavior,
        visibility: PollVisibilityPolicy,
    ) -> Result<Self, SubscriptionError> {
        if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&interval_ms)
            || jitter_basis_points > MAX_POLL_JITTER_BASIS_POINTS
        {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidPollFallback,
            ));
        }
        Ok(Self {
            interval_ms,
            jitter_basis_points,
            initial,
            visibility,
        })
    }

    /// Returns the base interval in milliseconds.
    #[must_use]
    pub const fn interval_ms(self) -> u64 {
        self.interval_ms
    }

    /// Returns jitter in basis points.
    #[must_use]
    pub const fn jitter_basis_points(self) -> u16 {
        self.jitter_basis_points
    }

    /// Returns the initial refresh policy.
    #[must_use]
    pub const fn initial(self) -> PollInitialBehavior {
        self.initial
    }

    /// Returns hidden-document behavior.
    #[must_use]
    pub const fn visibility(self) -> PollVisibilityPolicy {
        self.visibility
    }
}

/// Exact signed claims for one authorized asynchronous subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionClaims {
    stream: StreamName,
    protocol: u16,
    capability: CapabilityVersion,
    topics: BoundedTopics,
    events: BoundedEventNames,
    authorization_memo: AuthorizationMemo,
    baseline: StreamPosition,
    expires_at: UnixMillis,
    reconnect: ReconnectPolicy,
    fallback_poll: PollFallbackPolicy,
}

impl SubscriptionClaims {
    /// Creates exact bounded claims; issuance still requires a current trusted context.
    #[allow(
        clippy::too_many_arguments,
        reason = "the signed field set is an explicit protocol contract"
    )]
    pub fn new(
        stream: StreamName,
        protocol: u16,
        capability: CapabilityVersion,
        topics: BoundedTopics,
        events: BoundedEventNames,
        authorization_memo: AuthorizationMemo,
        baseline: StreamPosition,
        expires_at: UnixMillis,
        reconnect: ReconnectPolicy,
        fallback_poll: PollFallbackPolicy,
    ) -> Result<Self, SubscriptionError> {
        validate_protocol_and_reconnect(protocol, reconnect)?;
        Ok(Self {
            stream,
            protocol,
            capability,
            topics,
            events,
            authorization_memo,
            baseline,
            expires_at,
            reconnect,
            fallback_poll,
        })
    }

    /// Returns the registered stream identity.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns the independently versioned subscription protocol.
    #[must_use]
    pub const fn protocol(&self) -> u16 {
        self.protocol
    }

    /// Returns the negotiated capability version.
    #[must_use]
    pub const fn capability(&self) -> CapabilityVersion {
        self.capability
    }

    /// Returns exact trusted topics.
    #[must_use]
    pub const fn topics(&self) -> &BoundedTopics {
        &self.topics
    }

    /// Returns registered typed event contracts.
    #[must_use]
    pub const fn events(&self) -> &BoundedEventNames {
        &self.events
    }

    /// Returns the non-secret authorization context memo.
    #[must_use]
    pub const fn authorization_memo(&self) -> &AuthorizationMemo {
        &self.authorization_memo
    }

    /// Returns the authoritative initial stream baseline.
    #[must_use]
    pub const fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    /// Returns the exclusive descriptor expiry.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Returns bounded reconnect behavior.
    #[must_use]
    pub const fn reconnect(&self) -> ReconnectPolicy {
        self.reconnect
    }

    /// Returns authoritative hybrid fallback defaults.
    #[must_use]
    pub const fn fallback_poll(&self) -> PollFallbackPolicy {
        self.fallback_poll
    }
}

/// Signed non-secret descriptor wire value. It never contains a transport credential.
#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionDescriptor(String);

impl SubscriptionDescriptor {
    /// Parses only the bounded structural envelope; verification is separate.
    pub fn parse(value: &str) -> Result<Self, SubscriptionError> {
        parse_envelope(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the signed public descriptor wire value.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SubscriptionDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionDescriptor:redacted>")
    }
}

/// Secret bearer credential issued separately by the host transport adapter.
///
/// This type deliberately implements neither `Display` nor `Serialize`. The
/// authority-bearing accessor exists only for an authorization header or
/// equivalent protected transport-control field.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct TransportCredential(Zeroizing<Vec<u8>>);

impl TransportCredential {
    /// Takes bounded bytes minted by a trusted host credential provider.
    pub fn from_host_authority_bearer(bytes: Vec<u8>) -> Result<Self, SubscriptionError> {
        if bytes.len() < MIN_TRANSPORT_CREDENTIAL_BYTES
            || bytes.len() > MAX_TRANSPORT_CREDENTIAL_BYTES
        {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidCredential,
            ));
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Exposes bearer bytes to the host authorization transport only.
    ///
    /// Callers must never place these bytes in descriptors, snapshots, HTML,
    /// URLs, history, logs, traces, diagnostics, or action/model envelopes.
    #[must_use]
    pub fn expose_authorization_bearer(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl fmt::Debug for TransportCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TransportCredential:redacted>")
    }
}

/// Integrity-verified descriptor whose canonical claims are safe to inspect.
#[derive(Clone, Eq, PartialEq)]
pub struct VerifiedSubscriptionDescriptor {
    claims: SubscriptionClaims,
}

impl VerifiedSubscriptionDescriptor {
    pub(crate) const fn new(claims: SubscriptionClaims) -> Self {
        Self { claims }
    }

    /// Returns all exact verified claims.
    #[must_use]
    pub const fn claims(&self) -> &SubscriptionClaims {
        &self.claims
    }

    /// Returns the authoritative baseline.
    #[must_use]
    pub const fn baseline(&self) -> StreamPosition {
        self.claims.baseline()
    }

    /// Returns the exclusive descriptor expiry.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.claims.expires_at()
    }
}

impl fmt::Debug for VerifiedSubscriptionDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifiedSubscriptionDescriptor:redacted>")
    }
}

/// Purpose-separated signer and verifier over canonical exact-key claims.
pub struct SubscriptionDescriptorCodec {
    keys: SnapshotKeyRing,
}

impl SubscriptionDescriptorCodec {
    /// Creates a codec with the active and bounded overlapping verification keys.
    #[must_use]
    pub const fn new(keys: SnapshotKeyRing) -> Self {
        Self { keys }
    }

    /// Signs already-authorized claims with the subscription-v1 purpose.
    pub fn sign(
        &self,
        claims: &SubscriptionClaims,
        now: UnixMillis,
    ) -> Result<SubscriptionDescriptor, SubscriptionError> {
        validate_claims_at(claims, now)?;
        let body = encode_claims(claims)?;
        let signed = self
            .keys
            .sign(SnapshotPurpose::AsyncSubscriptionV1, &body, now)
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
        let wire = format!(
            "{DESCRIPTOR_VERSION}.{}.{}.{}",
            signed.key_id().as_str(),
            URL_SAFE_NO_PAD.encode(&body),
            signed.signature().to_base64url()
        );
        SubscriptionDescriptor::parse(&wire)
    }

    /// Verifies structure, signature, canonical exact fields, and exclusive expiry.
    pub fn verify(
        &self,
        descriptor: &SubscriptionDescriptor,
        now: UnixMillis,
    ) -> Result<VerifiedSubscriptionDescriptor, SubscriptionError> {
        let envelope = parse_envelope(descriptor.as_str())?;
        let body = decode_claim_body(envelope.body)?;
        self.keys
            .verify(
                &envelope.key_id,
                SnapshotPurpose::AsyncSubscriptionV1,
                &body,
                &envelope.signature,
                now,
            )
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
        let claims = decode_claims(&body)?;
        validate_claims_at(&claims, now)?;
        Ok(VerifiedSubscriptionDescriptor::new(claims))
    }
}

impl fmt::Debug for SubscriptionDescriptorCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionDescriptorCodec:redacted>")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaimsWire {
    authorization_memo: String,
    baseline: PositionWire,
    capability: u16,
    events: Vec<String>,
    expires_at: String,
    fallback_poll: PollWire,
    protocol: u16,
    reconnect: ReconnectWire,
    stream: String,
    topics: Vec<String>,
    v: u16,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PositionWire {
    epoch: String,
    sequence: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PollWire {
    initial: String,
    interval_ms: String,
    jitter_basis_points: u16,
    visibility: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReconnectWire {
    kind: String,
    maximum_attempts: Option<u8>,
}

impl ClaimsWire {
    fn from_claims(claims: &SubscriptionClaims) -> Self {
        let reconnect = match claims.reconnect {
            ReconnectPolicy::RefreshOnReconnect => ReconnectWire {
                kind: "refresh_on_reconnect".to_owned(),
                maximum_attempts: None,
            },
            ReconnectPolicy::ResumeOrRefresh { maximum_attempts } => ReconnectWire {
                kind: "resume_or_refresh".to_owned(),
                maximum_attempts: Some(maximum_attempts.get()),
            },
        };
        Self {
            authorization_memo: claims.authorization_memo.as_str().to_owned(),
            baseline: PositionWire {
                epoch: claims.baseline.epoch().get().to_string(),
                sequence: claims.baseline.sequence().get().to_string(),
            },
            capability: claims.capability.get(),
            events: claims
                .events
                .as_slice()
                .iter()
                .map(|event| event.as_str().to_owned())
                .collect(),
            expires_at: claims.expires_at.get().to_string(),
            fallback_poll: PollWire {
                initial: match claims.fallback_poll.initial {
                    PollInitialBehavior::Immediate => "immediate",
                    PollInitialBehavior::AfterInterval => "after_interval",
                }
                .to_owned(),
                interval_ms: claims.fallback_poll.interval_ms.to_string(),
                jitter_basis_points: claims.fallback_poll.jitter_basis_points,
                visibility: match claims.fallback_poll.visibility {
                    PollVisibilityPolicy::PauseWhenHidden => "pause_when_hidden",
                    PollVisibilityPolicy::ContinueWhenHidden => "continue_when_hidden",
                }
                .to_owned(),
            },
            protocol: claims.protocol,
            reconnect,
            stream: claims.stream.as_str().to_owned(),
            topics: claims
                .topics
                .as_slice()
                .iter()
                .map(|topic| topic.as_str().to_owned())
                .collect(),
            v: DESCRIPTOR_SCHEMA_VERSION,
        }
    }

    fn into_claims(self) -> Result<SubscriptionClaims, SubscriptionError> {
        if self.v != DESCRIPTOR_SCHEMA_VERSION {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidDescriptor,
            ));
        }
        let reconnect = match (
            self.reconnect.kind.as_str(),
            self.reconnect.maximum_attempts,
        ) {
            ("refresh_on_reconnect", None) => ReconnectPolicy::RefreshOnReconnect,
            ("resume_or_refresh", Some(value)) => ReconnectPolicy::ResumeOrRefresh {
                maximum_attempts: std::num::NonZeroU8::new(value).ok_or_else(|| {
                    SubscriptionError::new(SubscriptionErrorKind::InvalidReconnectPolicy)
                })?,
            },
            _ => {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::InvalidReconnectPolicy,
                ));
            }
        };
        let initial = match self.fallback_poll.initial.as_str() {
            "immediate" => PollInitialBehavior::Immediate,
            "after_interval" => PollInitialBehavior::AfterInterval,
            _ => {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::InvalidPollFallback,
                ));
            }
        };
        let visibility = match self.fallback_poll.visibility.as_str() {
            "pause_when_hidden" => PollVisibilityPolicy::PauseWhenHidden,
            "continue_when_hidden" => PollVisibilityPolicy::ContinueWhenHidden,
            _ => {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::InvalidPollFallback,
                ));
            }
        };
        let events = self
            .events
            .iter()
            .map(|event| BrowserOperationName::parse(event))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
        let topics = self
            .topics
            .iter()
            .map(|topic| TopicName::parse(topic))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
        SubscriptionClaims::new(
            StreamName::parse(&self.stream)
                .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
            self.protocol,
            CapabilityVersion::new(self.capability)?,
            BoundedTopics::new(topics)
                .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
            BoundedEventNames::new(events)
                .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
            AuthorizationMemo::parse(&self.authorization_memo)?,
            StreamPosition::new(
                StreamEpoch::new(parse_counter(&self.baseline.epoch)?),
                StreamSequence::new(parse_counter(&self.baseline.sequence)?),
            ),
            UnixMillis::parse(&self.expires_at)
                .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
            reconnect,
            PollFallbackPolicy::new(
                parse_counter(&self.fallback_poll.interval_ms)?,
                self.fallback_poll.jitter_basis_points,
                initial,
                visibility,
            )?,
        )
    }
}

struct DescriptorEnvelope<'a> {
    key_id: KeyId,
    body: &'a str,
    signature: SnapshotSignature,
}

fn parse_envelope(value: &str) -> Result<DescriptorEnvelope<'_>, SubscriptionError> {
    if value.is_empty() || value.len() > MAX_DESCRIPTOR_BYTES || !value.is_ascii() {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    let mut parts = value.split('.');
    let version = parts.next();
    let key_id = parts.next();
    let body = parts.next();
    let signature = parts.next();
    if version != Some(DESCRIPTOR_VERSION)
        || parts.next().is_some()
        || key_id.is_none_or(str::is_empty)
        || body.is_none_or(str::is_empty)
        || signature.is_none_or(str::is_empty)
    {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    let body = body.unwrap_or_default();
    if body.len() > MAX_ENCODED_CLAIMS_BYTES
        || body.contains('=')
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    Ok(DescriptorEnvelope {
        key_id: KeyId::parse(key_id.unwrap_or_default())
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
        body,
        signature: SnapshotSignature::parse(signature.unwrap_or_default())
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?,
    })
}

fn claims_limits() -> Result<InputLimits, SubscriptionError> {
    InputLimits::new(MAX_CLAIMS_BYTES, 6, 256, 512)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))
}

fn encode_claims(claims: &SubscriptionClaims) -> Result<Vec<u8>, SubscriptionError> {
    let serde_value = serde_json::to_value(ClaimsWire::from_claims(claims))
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    let canonical = CanonicalValue::from_serde_value(serde_value)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    to_canonical_bytes(&canonical, &claims_limits()?)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))
}

fn decode_claim_body(encoded: &str) -> Result<Vec<u8>, SubscriptionError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    if decoded.len() > MAX_CLAIMS_BYTES || URL_SAFE_NO_PAD.encode(&decoded) != encoded {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    Ok(decoded)
}

fn decode_claims(body: &[u8]) -> Result<SubscriptionClaims, SubscriptionError> {
    let canonical = parse_canonical_value(body, &claims_limits()?)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    let CanonicalValue::Object(fields) = &canonical else {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    };
    if fields.len() != CLAIM_KEYS.len() || !CLAIM_KEYS.iter().all(|key| fields.contains_key(*key)) {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    let encoded = to_canonical_bytes(&canonical, &claims_limits()?)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    if encoded != body {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    let serde_value = canonical
        .to_serde_value()
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    let wire: ClaimsWire = serde_json::from_value(serde_value)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))?;
    wire.into_claims()
}

fn validate_claims_at(
    claims: &SubscriptionClaims,
    now: UnixMillis,
) -> Result<(), SubscriptionError> {
    validate_protocol_and_reconnect(claims.protocol, claims.reconnect)?;
    if claims.expires_at <= now {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::DescriptorExpired,
        ));
    }
    if claims.expires_at.get().saturating_sub(now.get()) > MAX_SUBSCRIPTION_LIFETIME_MS {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    Ok(())
}

fn validate_protocol_and_reconnect(
    protocol: u16,
    reconnect: ReconnectPolicy,
) -> Result<(), SubscriptionError> {
    if protocol != ASYNC_SUBSCRIPTION_PROTOCOL_V1 {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::UnsupportedProtocol,
        ));
    }
    if matches!(
        reconnect,
        ReconnectPolicy::ResumeOrRefresh { maximum_attempts }
            if maximum_attempts.get() > MAX_RECONNECT_ATTEMPTS
    ) {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidReconnectPolicy,
        ));
    }
    Ok(())
}

fn parse_counter(value: &str) -> Result<u64, SubscriptionError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    value
        .parse()
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))
}
