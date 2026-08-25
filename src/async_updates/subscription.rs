//! Canonical signed subscription descriptions and separately secret credentials.

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU8, NonZeroU16};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::crypto::{SnapshotKeyRing, SnapshotPurpose, SnapshotSignature};
use crate::identity::{BrowserOperationName, IslandSlot, KeyId, UnixMillis};
use crate::limits::InputLimits;
use crate::metadata::{EventMetadata, PayloadContractIdentity};

use super::{
    BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventCyclePolicy, EventOrder, EventSource,
    EventTarget, MAX_EVENT_FANOUT, MAX_SUBSCRIPTION_EVENTS, ReconnectPolicy, StreamName, TopicName,
};

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
const MAX_DESCRIPTOR_BYTES: usize = 131_072;
const MAX_ENCODED_CLAIMS_BYTES: usize = 100_000;
const MAX_CLAIMS_BYTES: usize = 65_536;
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
    /// Verified claims do not match current identity, component contract, stream, topics, or events.
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

/// Security- and compatibility-significant fields of one registered stream event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionEventContract {
    name: BrowserOperationName,
    version: u16,
    payload_contract: PayloadContractIdentity,
    schema: BrowserPayloadSchema,
    source: EventSource,
    targets: BoundedTargets,
    order: EventOrder,
    cycle: EventCyclePolicy,
    maximum_fanout: NonZeroU16,
}

impl SubscriptionEventContract {
    /// Copies the stable wire contract from current registry metadata.
    pub fn from_registered(metadata: &EventMetadata) -> Result<Self, SubscriptionError> {
        Self::new(
            metadata.name().clone(),
            metadata.version(),
            metadata.payload_contract().clone(),
            metadata.schema(),
            metadata.source(),
            metadata.targets().clone(),
            metadata.order(),
            metadata.cycle(),
            metadata.maximum_fanout(),
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the event field set is the signed compatibility contract"
    )]
    fn new(
        name: BrowserOperationName,
        version: u16,
        payload_contract: PayloadContractIdentity,
        schema: BrowserPayloadSchema,
        source: EventSource,
        targets: BoundedTargets,
        order: EventOrder,
        cycle: EventCyclePolicy,
        maximum_fanout: NonZeroU16,
    ) -> Result<Self, SubscriptionError> {
        if version == 0
            || source != EventSource::Stream
            || maximum_fanout.get() > MAX_EVENT_FANOUT
            || usize::from(maximum_fanout.get()) < targets.as_slice().len()
        {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidDescriptor,
            ));
        }
        Ok(Self {
            name,
            version,
            payload_contract,
            schema,
            source,
            targets,
            order,
            cycle,
            maximum_fanout,
        })
    }

    /// Returns the registered browser event identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the payload contract version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stable payload contract identity.
    #[must_use]
    pub const fn payload_contract(&self) -> &PayloadContractIdentity {
        &self.payload_contract
    }

    /// Returns the browser payload root schema.
    #[must_use]
    pub const fn schema(&self) -> BrowserPayloadSchema {
        self.schema
    }

    /// Returns the trusted event source.
    #[must_use]
    pub const fn source(&self) -> EventSource {
        self.source
    }

    /// Returns the canonical propagation targets.
    #[must_use]
    pub const fn targets(&self) -> &BoundedTargets {
        &self.targets
    }

    /// Returns the delivery ordering contract.
    #[must_use]
    pub const fn order(&self) -> EventOrder {
        self.order
    }

    /// Returns the delivery cycle-prevention contract.
    #[must_use]
    pub const fn cycle(&self) -> EventCyclePolicy {
        self.cycle
    }

    /// Returns the maximum event delivery fanout.
    #[must_use]
    pub const fn maximum_fanout(&self) -> NonZeroU16 {
        self.maximum_fanout
    }
}

/// Canonically sorted, duplicate-free full stream event contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedEventContracts(Vec<SubscriptionEventContract>);

impl BoundedEventContracts {
    /// Sorts and validates the bounded full contract set.
    pub fn new(mut events: Vec<SubscriptionEventContract>) -> Result<Self, SubscriptionError> {
        if events.is_empty() || events.len() > MAX_SUBSCRIPTION_EVENTS {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidDescriptor,
            ));
        }
        events.sort_by(|left, right| left.name().cmp(right.name()));
        if events
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidDescriptor,
            ));
        }
        Ok(Self(events))
    }

    /// Returns full contracts in canonical event-name order.
    #[must_use]
    pub fn as_slice(&self) -> &[SubscriptionEventContract] {
        &self.0
    }
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
    events: BoundedEventContracts,
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
        events: BoundedEventContracts,
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
    pub const fn events(&self) -> &BoundedEventContracts {
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
    events: Vec<EventContractWire>,
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
struct EventContractWire {
    cycle: EventCycleWire,
    maximum_fanout: u16,
    name: String,
    order: String,
    payload_contract: String,
    schema: String,
    source: String,
    targets: Vec<EventTargetWire>,
    version: u16,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EventCycleWire {
    kind: String,
    maximum_hops: Option<u8>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EventTargetWire {
    kind: String,
    value: Option<String>,
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
                .map(EventContractWire::from_contract)
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
            .into_iter()
            .map(EventContractWire::into_contract)
            .collect::<Result<Vec<_>, _>>()?;
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
            BoundedEventContracts::new(events)?,
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

impl EventContractWire {
    fn from_contract(contract: &SubscriptionEventContract) -> Self {
        Self {
            cycle: match contract.cycle() {
                EventCyclePolicy::ForbidRepeatedIsland => EventCycleWire {
                    kind: "forbid_repeated_island".to_owned(),
                    maximum_hops: None,
                },
                EventCyclePolicy::MaximumHops(maximum_hops) => EventCycleWire {
                    kind: "maximum_hops".to_owned(),
                    maximum_hops: Some(maximum_hops.get()),
                },
            },
            maximum_fanout: contract.maximum_fanout().get(),
            name: contract.name().as_str().to_owned(),
            order: match contract.order() {
                EventOrder::PerSourceSequence => "per_source_sequence",
            }
            .to_owned(),
            payload_contract: contract.payload_contract().as_str().to_owned(),
            schema: schema_name(contract.schema()).to_owned(),
            source: match contract.source() {
                EventSource::Component => "component",
                EventSource::Stream => "stream",
            }
            .to_owned(),
            targets: contract
                .targets()
                .as_slice()
                .iter()
                .map(EventTargetWire::from_target)
                .collect(),
            version: contract.version(),
        }
    }

    fn into_contract(self) -> Result<SubscriptionEventContract, SubscriptionError> {
        let schema = match self.schema.as_str() {
            "json" => BrowserPayloadSchema::Json,
            "null" => BrowserPayloadSchema::Null,
            "boolean" => BrowserPayloadSchema::Boolean,
            "i64" => BrowserPayloadSchema::I64,
            "u64" => BrowserPayloadSchema::U64,
            "f64" => BrowserPayloadSchema::F64,
            "string" => BrowserPayloadSchema::String,
            _ => return Err(invalid_descriptor()),
        };
        let source = match self.source.as_str() {
            "component" => EventSource::Component,
            "stream" => EventSource::Stream,
            _ => return Err(invalid_descriptor()),
        };
        let order = match self.order.as_str() {
            "per_source_sequence" => EventOrder::PerSourceSequence,
            _ => return Err(invalid_descriptor()),
        };
        let cycle = match (self.cycle.kind.as_str(), self.cycle.maximum_hops) {
            ("forbid_repeated_island", None) => EventCyclePolicy::ForbidRepeatedIsland,
            ("maximum_hops", Some(maximum_hops)) => EventCyclePolicy::MaximumHops(
                NonZeroU8::new(maximum_hops).ok_or_else(invalid_descriptor)?,
            ),
            _ => return Err(invalid_descriptor()),
        };
        let targets = self
            .targets
            .into_iter()
            .map(EventTargetWire::into_target)
            .collect::<Result<Vec<_>, _>>()?;
        SubscriptionEventContract::new(
            BrowserOperationName::parse(&self.name).map_err(|_| invalid_descriptor())?,
            self.version,
            PayloadContractIdentity::parse(&self.payload_contract)
                .map_err(|_| invalid_descriptor())?,
            schema,
            source,
            BoundedTargets::new(targets).map_err(|_| invalid_descriptor())?,
            order,
            cycle,
            NonZeroU16::new(self.maximum_fanout).ok_or_else(invalid_descriptor)?,
        )
    }
}

impl EventTargetWire {
    fn from_target(target: &EventTarget) -> Self {
        match target {
            EventTarget::SelfIsland => Self::without_value("self_island"),
            EventTarget::Parent => Self::without_value("parent"),
            EventTarget::Child => Self::without_value("child"),
            EventTarget::NamedIsland(slot) => Self::with_value("named_island", slot.as_str()),
            EventTarget::Document => Self::without_value("document"),
            EventTarget::Browser(listener) => Self::with_value("browser", listener.as_str()),
        }
    }

    fn without_value(kind: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            value: None,
        }
    }

    fn with_value(kind: &str, value: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            value: Some(value.to_owned()),
        }
    }

    fn into_target(self) -> Result<EventTarget, SubscriptionError> {
        match (self.kind.as_str(), self.value) {
            ("self_island", None) => Ok(EventTarget::SelfIsland),
            ("parent", None) => Ok(EventTarget::Parent),
            ("child", None) => Ok(EventTarget::Child),
            ("named_island", Some(value)) => IslandSlot::parse(&value)
                .map(EventTarget::NamedIsland)
                .map_err(|_| invalid_descriptor()),
            ("document", None) => Ok(EventTarget::Document),
            ("browser", Some(value)) => BrowserOperationName::parse(&value)
                .map(EventTarget::Browser)
                .map_err(|_| invalid_descriptor()),
            _ => Err(invalid_descriptor()),
        }
    }
}

const fn schema_name(schema: BrowserPayloadSchema) -> &'static str {
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

const fn invalid_descriptor() -> SubscriptionError {
    SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor)
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
    InputLimits::new(MAX_CLAIMS_BYTES, 8, 4_096, 8_192)
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
    let claims = wire.into_claims()?;
    if encode_claims(&claims)? != body {
        return Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidDescriptor,
        ));
    }
    Ok(claims)
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
