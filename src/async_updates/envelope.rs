//! Canonical, bounded asynchronous event envelopes.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::canonical::{
    CanonicalErrorKind, CanonicalValue, MAX_SAFE_INTEGER, parse_canonical_value, to_canonical_bytes,
};
use crate::identity::{BrowserOperationName, ContentDigest, IslandSlot, UnixMillis};
use crate::limits::{InputLimits, LimitConfigurationError};

use super::{
    AuthorizationMemo, AuthorizedSubscription, BoundedEventContracts, BrowserPayloadSchema,
    DocumentAuthorizationScope, EventTarget, StreamEpoch, StreamName, StreamPosition,
    StreamSequence, SubscriptionBinding, SubscriptionEventContract, VerifiedSubscriptionDescriptor,
};

/// Independently versioned asynchronous event-envelope majors supported here.
pub const SUPPORTED_ASYNC_PROTOCOL_VERSIONS: &[u16] = &[1];

const MAX_SUBSCRIPTION_ID_BYTES: usize = 32;
const MIN_SUBSCRIPTION_ID_BYTES: usize = 16;
const MAX_PRESENTATION_SIGNALS: usize = 64;
/// Maximum total array elements plus object members in async protocol v1.
pub const MAX_ASYNC_ENVELOPE_ENTRIES: usize = 1_024;
const ENVELOPE_KEYS: [&str; 5] = [
    "payload",
    "position",
    "protocol_version",
    "stream",
    "subscription",
];
const POSITION_KEYS: [&str; 2] = ["epoch", "sequence"];

/// Why an asynchronous envelope was rejected before dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncEnvelopeErrorKind {
    /// Raw envelope bytes exceeded the configured boundary.
    TooLarge,
    /// JSON nesting exceeded the configured boundary.
    TooDeep,
    /// JSON collection entries exceeded the configured boundary.
    TooManyEntries,
    /// A decoded string or key exceeded the configured boundary.
    StringTooLong,
    /// An object repeated the same field name.
    DuplicateField,
    /// Input was valid JSON but not the required canonical representation.
    NonCanonical,
    /// The envelope shape, identity, or position was invalid.
    InvalidEnvelope,
    /// The independent asynchronous protocol major is unsupported.
    UnsupportedProtocol,
    /// The envelope names a subscription outside the selected active membership.
    SubscriptionMismatch,
    /// The envelope stream differs from the selected registered subscription.
    StreamMismatch,
    /// Canonical payload bytes exceeded the configured payload boundary.
    PayloadTooLarge,
    /// The payload kind is outside the closed asynchronous authority surface.
    UnsupportedPayload,
    /// A known payload kind had malformed or inconsistent fields.
    InvalidPayload,
    /// A typed event or presentation signal did not match current registration.
    UnregisteredPayload,
    /// The signed subscription descriptor reached its exclusive expiry.
    MembershipExpired,
}

impl AsyncEnvelopeErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "async_envelope_too_large",
            Self::TooDeep => "async_envelope_too_deep",
            Self::TooManyEntries => "too_many_async_envelope_entries",
            Self::StringTooLong => "async_envelope_string_too_long",
            Self::DuplicateField => "duplicate_async_envelope_field",
            Self::NonCanonical => "noncanonical_async_envelope",
            Self::InvalidEnvelope => "invalid_async_envelope",
            Self::UnsupportedProtocol => "unsupported_async_protocol",
            Self::SubscriptionMismatch => "async_subscription_mismatch",
            Self::StreamMismatch => "async_stream_mismatch",
            Self::PayloadTooLarge => "async_payload_too_large",
            Self::UnsupportedPayload => "unsupported_async_payload",
            Self::InvalidPayload => "invalid_async_payload",
            Self::UnregisteredPayload => "unregistered_async_payload",
            Self::MembershipExpired => "async_membership_expired",
        }
    }
}

/// Redacted asynchronous-envelope rejection.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AsyncEnvelopeError {
    kind: AsyncEnvelopeErrorKind,
}

impl AsyncEnvelopeError {
    const fn new(kind: AsyncEnvelopeErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed safe rejection reason.
    #[must_use]
    pub const fn kind(self) -> AsyncEnvelopeErrorKind {
        self.kind
    }
}

impl fmt::Display for AsyncEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for AsyncEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for AsyncEnvelopeError {}

/// Validated byte, structure, string, and payload limits for async protocol v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncCodecLimits {
    input: InputLimits,
    max_payload_bytes: usize,
}

impl AsyncCodecLimits {
    /// Creates a coherent nonzero profile whose payload boundary fits the envelope boundary.
    pub fn new(
        max_bytes: usize,
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
        max_payload_bytes: usize,
    ) -> Result<Self, LimitConfigurationError> {
        let input = InputLimits::new(max_bytes, max_depth, max_entries, max_string_bytes)?;
        if max_payload_bytes == 0 || max_payload_bytes > max_bytes {
            return Err(LimitConfigurationError);
        }
        Ok(Self {
            input,
            max_payload_bytes,
        })
    }

    /// Returns the locked protocol-v1 envelope profile shared by the v4 corpus.
    #[must_use]
    pub fn v1() -> Self {
        match Self::new(65_536, 8, MAX_ASYNC_ENVELOPE_ENTRIES, 4_096, 32_768) {
            Ok(limits) => limits,
            Err(_) => unreachable!("locked async limits are below engine ceilings"),
        }
    }

    const fn input(self) -> InputLimits {
        self.input
    }

    const fn max_payload_bytes(self) -> usize {
        self.max_payload_bytes
    }
}

/// Opaque bounded identity for one logical subscription membership.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SubscriptionId(Vec<u8>);

impl SubscriptionId {
    /// Minimum canonical unpadded base64url length for a 128-bit identity.
    pub const MIN_ENCODED_LEN: usize = 22;

    /// Maximum canonical unpadded base64url length for a 256-bit identity.
    pub const MAX_ENCODED_LEN: usize = 43;

    /// Constructs an identity from trusted server bytes.
    pub fn from_bytes(value: &[u8]) -> Result<Self, AsyncEnvelopeError> {
        if !(MIN_SUBSCRIPTION_ID_BYTES..=MAX_SUBSCRIPTION_ID_BYTES).contains(&value.len()) {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::InvalidEnvelope,
            ));
        }
        Ok(Self(value.to_vec()))
    }

    /// Parses canonical unpadded base64url subscription identity.
    pub fn parse(value: &str) -> Result<Self, AsyncEnvelopeError> {
        if !(Self::MIN_ENCODED_LEN..=Self::MAX_ENCODED_LEN).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::InvalidEnvelope,
            ));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::InvalidEnvelope))?;
        let identity = Self::from_bytes(&bytes)?;
        if identity.to_base64url() != value {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::InvalidEnvelope,
            ));
        }
        Ok(identity)
    }

    /// Returns canonical unpadded base64url for wire routing.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.0)
    }
}

impl fmt::Debug for SubscriptionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionId>")
    }
}

/// Registered schema for one presentation-only local-signal update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationSignalContract {
    name: BrowserOperationName,
    schema: BrowserPayloadSchema,
}

impl PresentationSignalContract {
    /// Creates one trusted signal contract from generated metadata.
    #[must_use]
    pub const fn new(name: BrowserOperationName, schema: BrowserPayloadSchema) -> Self {
        Self { name, schema }
    }

    /// Returns the registered signal identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the declared JSON root schema.
    #[must_use]
    pub const fn schema(&self) -> BrowserPayloadSchema {
        self.schema
    }
}

/// Canonically sorted, duplicate-free presentation signal contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedPresentationSignalContracts(Vec<PresentationSignalContract>);

impl BoundedPresentationSignalContracts {
    /// Sorts and validates the complete bounded signal contract set.
    pub fn new(mut signals: Vec<PresentationSignalContract>) -> Result<Self, AsyncEnvelopeError> {
        if signals.len() > MAX_PRESENTATION_SIGNALS {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        signals.sort_by(|left, right| left.name().cmp(right.name()));
        if signals
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        Ok(Self(signals))
    }

    fn find(&self, name: &BrowserOperationName) -> Option<&PresentationSignalContract> {
        self.0
            .binary_search_by(|candidate| candidate.name().cmp(name))
            .ok()
            .map(|index| &self.0[index])
    }
}

/// Exact authorized subscription and proposed membership supplied to the host registry.
#[derive(Clone, Copy)]
pub struct AsyncMembershipRequest<'a> {
    verified: &'a VerifiedSubscriptionDescriptor,
    subscription: &'a SubscriptionId,
    envelope: Option<&'a AsyncEnvelope>,
    binding: Option<&'a SubscriptionBinding>,
    document_scope: Option<&'a DocumentAuthorizationScope>,
}

/// Exact bounded replay transcript supplied to one atomic host-registry check.
#[derive(Clone, Copy)]
pub struct AsyncReplayMembershipRequest<'a> {
    verified: &'a VerifiedSubscriptionDescriptor,
    subscription: &'a SubscriptionId,
    envelopes: &'a [AsyncEnvelope],
    binding: &'a SubscriptionBinding,
    document_scope: &'a DocumentAuthorizationScope,
}

impl<'a> AsyncReplayMembershipRequest<'a> {
    /// Returns the Task 2 integrity-verified subscription descriptor.
    #[must_use]
    pub const fn verified(self) -> &'a VerifiedSubscriptionDescriptor {
        self.verified
    }

    /// Returns the exact logical membership identity.
    #[must_use]
    pub const fn subscription(self) -> &'a SubscriptionId {
        self.subscription
    }

    /// Returns the complete bounded transcript being admitted atomically.
    #[must_use]
    pub const fn envelopes(self) -> &'a [AsyncEnvelope] {
        self.envelopes
    }

    /// Returns the exact signed-descriptor binding.
    #[must_use]
    pub const fn binding(self) -> &'a SubscriptionBinding {
        self.binding
    }

    /// Returns the exact document authorization scope.
    #[must_use]
    pub const fn document_scope(self) -> &'a DocumentAuthorizationScope {
        self.document_scope
    }
}

impl fmt::Debug for AsyncReplayMembershipRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncReplayMembershipRequest:redacted>")
    }
}

impl<'a> AsyncMembershipRequest<'a> {
    /// Returns the Task 2 integrity-verified subscription descriptor.
    #[must_use]
    pub const fn verified(self) -> &'a VerifiedSubscriptionDescriptor {
        self.verified
    }

    /// Returns the logical membership identity requiring active validation.
    #[must_use]
    pub const fn subscription(self) -> &'a SubscriptionId {
        self.subscription
    }

    /// Returns the exact server-authored envelope at a delivery admission boundary.
    #[must_use]
    pub const fn envelope(self) -> Option<&'a AsyncEnvelope> {
        self.envelope
    }

    /// Returns the exact descriptor binding at a delivery admission boundary.
    #[must_use]
    pub const fn binding(self) -> Option<&'a SubscriptionBinding> {
        self.binding
    }

    /// Returns the exact document scope at a delivery admission boundary.
    #[must_use]
    pub const fn document_scope(self) -> Option<&'a DocumentAuthorizationScope> {
        self.document_scope
    }
}

impl fmt::Debug for AsyncMembershipRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncMembershipRequest:redacted>")
    }
}

/// Host-owned active-membership and current-registry authority.
///
/// Task 3 defines only this validation hook. Physical transport membership and
/// multiplexing remain owned by Task 4.
pub trait AsyncMembershipRegistryPort: Send + Sync {
    /// Atomically validates active membership and supplies one current registry snapshot.
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    );

    /// Atomically validates one complete bounded replay transcript.
    ///
    /// The default denies admission. Hosts that support replay must provide one
    /// coherent registry snapshot and one resolved fanout proof per envelope.
    fn validate_replay_current(
        &self,
        request: AsyncReplayMembershipRequest<'_>,
        validation: &mut AsyncReplayMembershipValidation<'_>,
    ) {
        let _ = (request, validation);
    }
}

/// Trusted current recipient resolution for one registered browser event.
#[derive(Clone, Eq, PartialEq)]
pub struct ResolvedEventFanout {
    recipients: NonZeroU16,
    target_scope: ContentDigest,
}

impl ResolvedEventFanout {
    /// Seals host-resolved recipients and an exact current target-set digest.
    #[must_use]
    pub const fn from_host(recipients: NonZeroU16, target_scope: ContentDigest) -> Self {
        Self {
            recipients,
            target_scope,
        }
    }

    /// Returns the host-resolved current recipient count.
    #[must_use]
    pub const fn recipients(&self) -> NonZeroU16 {
        self.recipients
    }

    pub(crate) const fn target_scope(&self) -> &ContentDigest {
        &self.target_scope
    }
}

impl fmt::Debug for ResolvedEventFanout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedEventFanout")
            .field("recipients", &self.recipients)
            .field("target_scope", &"<redacted>")
            .finish()
    }
}

struct ValidatedMembership {
    stream: StreamName,
    events: BoundedEventContracts,
    presentation_signals: BoundedPresentationSignalContracts,
    resolved_event: Option<ResolvedEventFanout>,
}

struct ValidatedReplayMembership {
    stream: StreamName,
    events: BoundedEventContracts,
    presentation_signals: BoundedPresentationSignalContracts,
    resolved_events: Vec<Option<ResolvedEventFanout>>,
}

/// Framework-owned sink that seals exactly one host-validated registry snapshot.
pub struct AsyncMembershipValidation<'a> {
    verified: &'a VerifiedSubscriptionDescriptor,
    expected_envelope: Option<&'a AsyncEnvelope>,
    expected_document_scope: Option<&'a DocumentAuthorizationScope>,
    candidate: Option<ValidatedMembership>,
    rejected: Option<AsyncEnvelopeErrorKind>,
}

impl AsyncMembershipValidation<'_> {
    /// Accepts one atomic current snapshot from the host-owned membership port.
    ///
    /// The framework independently compares stream and full event contracts to
    /// the Task 2 connect-authorized claims. A second acceptance fails closed.
    pub fn accept_current(
        &mut self,
        stream: &StreamName,
        events: &BoundedEventContracts,
        presentation_signals: &BoundedPresentationSignalContracts,
    ) -> bool {
        if self.candidate.is_some() || self.rejected.is_some() {
            self.candidate = None;
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        let claims = self.verified.claims();
        if stream != claims.stream() {
            self.rejected = Some(AsyncEnvelopeErrorKind::StreamMismatch);
            return false;
        }
        if events != claims.events() {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        if self.expected_envelope.is_some() || self.expected_document_scope.is_some() {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        self.candidate = Some(ValidatedMembership {
            stream: stream.clone(),
            events: events.clone(),
            presentation_signals: presentation_signals.clone(),
            resolved_event: None,
        });
        true
    }

    /// Accepts one fresh delivery snapshot including exact identity and host fanout.
    pub fn accept_delivery_current(
        &mut self,
        stream: &StreamName,
        events: &BoundedEventContracts,
        presentation_signals: &BoundedPresentationSignalContracts,
        authorization_memo: &AuthorizationMemo,
        document_scope: &DocumentAuthorizationScope,
        resolved_event: Option<ResolvedEventFanout>,
    ) -> bool {
        if self.candidate.is_some() || self.rejected.is_some() {
            self.candidate = None;
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        let Some(envelope) = self.expected_envelope else {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        };
        let claims = self.verified.claims();
        let exact_current = stream == claims.stream()
            && events == claims.events()
            && authorization_memo == claims.authorization_memo()
            && self.expected_document_scope == Some(document_scope);
        if !exact_current {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        let resolved_is_valid = resolved_event_is_valid(envelope, resolved_event.as_ref());
        if !resolved_is_valid {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        self.candidate = Some(ValidatedMembership {
            stream: stream.clone(),
            events: events.clone(),
            presentation_signals: presentation_signals.clone(),
            resolved_event,
        });
        true
    }

    /// Accepts one current exact membership scope without a payload delivery.
    pub fn accept_scope_current(
        &mut self,
        stream: &StreamName,
        events: &BoundedEventContracts,
        presentation_signals: &BoundedPresentationSignalContracts,
        authorization_memo: &AuthorizationMemo,
        document_scope: &DocumentAuthorizationScope,
    ) -> bool {
        if self.candidate.is_some() || self.rejected.is_some() {
            self.candidate = None;
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        let claims = self.verified.claims();
        if self.expected_envelope.is_some()
            || stream != claims.stream()
            || events != claims.events()
            || authorization_memo != claims.authorization_memo()
            || self.expected_document_scope != Some(document_scope)
        {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        self.candidate = Some(ValidatedMembership {
            stream: stream.clone(),
            events: events.clone(),
            presentation_signals: presentation_signals.clone(),
            resolved_event: None,
        });
        true
    }

    fn finish(self) -> Result<ValidatedMembership, AsyncEnvelopeError> {
        self.candidate.ok_or_else(|| {
            AsyncEnvelopeError::new(
                self.rejected
                    .unwrap_or(AsyncEnvelopeErrorKind::SubscriptionMismatch),
            )
        })
    }
}

fn resolved_event_is_valid(
    envelope: &AsyncEnvelope,
    resolved_event: Option<&ResolvedEventFanout>,
) -> bool {
    match envelope.payload() {
        AsyncPayload::BrowserEvent(event) => resolved_event.is_some_and(|resolved| {
            let recipients = resolved.recipients().get();
            recipients <= event.maximum_fanout().get()
                && match event.target() {
                    EventTarget::SelfIsland | EventTarget::Parent | EventTarget::NamedIsland(_) => {
                        recipients == 1
                    }
                    EventTarget::Child | EventTarget::Document | EventTarget::Browser(_) => true,
                }
        }),
        _ => resolved_event.is_none(),
    }
}

impl fmt::Debug for AsyncMembershipValidation<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncMembershipValidation:redacted>")
    }
}

/// Framework-owned sink for one atomic replay-registry snapshot.
pub struct AsyncReplayMembershipValidation<'a> {
    verified: &'a VerifiedSubscriptionDescriptor,
    expected_envelopes: &'a [AsyncEnvelope],
    expected_document_scope: &'a DocumentAuthorizationScope,
    candidate: Option<ValidatedReplayMembership>,
    rejected: Option<AsyncEnvelopeErrorKind>,
}

impl AsyncReplayMembershipValidation<'_> {
    /// Accepts the complete current contract and exact fanout proof vector once.
    pub fn accept_current(
        &mut self,
        stream: &StreamName,
        events: &BoundedEventContracts,
        presentation_signals: &BoundedPresentationSignalContracts,
        authorization_memo: &AuthorizationMemo,
        document_scope: &DocumentAuthorizationScope,
        resolved_events: &[Option<ResolvedEventFanout>],
    ) -> bool {
        if self.candidate.is_some() || self.rejected.is_some() {
            self.candidate = None;
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        let claims = self.verified.claims();
        if stream != claims.stream()
            || events != claims.events()
            || authorization_memo != claims.authorization_memo()
            || document_scope != self.expected_document_scope
            || resolved_events.len() != self.expected_envelopes.len()
            || self
                .expected_envelopes
                .iter()
                .zip(resolved_events)
                .any(|(envelope, resolved)| !resolved_event_is_valid(envelope, resolved.as_ref()))
        {
            self.rejected = Some(AsyncEnvelopeErrorKind::UnregisteredPayload);
            return false;
        }
        self.candidate = Some(ValidatedReplayMembership {
            stream: stream.clone(),
            events: events.clone(),
            presentation_signals: presentation_signals.clone(),
            resolved_events: resolved_events.to_vec(),
        });
        true
    }

    fn finish(self) -> Result<ValidatedReplayMembership, AsyncEnvelopeError> {
        self.candidate.ok_or_else(|| {
            AsyncEnvelopeError::new(
                self.rejected
                    .unwrap_or(AsyncEnvelopeErrorKind::SubscriptionMismatch),
            )
        })
    }
}

impl fmt::Debug for AsyncReplayMembershipValidation<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncReplayMembershipValidation:redacted>")
    }
}

/// Sealed active-membership and registered-payload context required for decode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsyncEnvelopeContext {
    verified: VerifiedSubscriptionDescriptor,
    subscription: SubscriptionId,
    stream: StreamName,
    authoritative_baseline: StreamPosition,
    events: BoundedEventContracts,
    presentation_signals: BoundedPresentationSignalContracts,
}

impl AsyncEnvelopeContext {
    /// Validates Task 2 authorization against active membership and current registry.
    pub fn from_authorized(
        authorized: &AuthorizedSubscription,
        subscription: SubscriptionId,
        registry: &dyn AsyncMembershipRegistryPort,
    ) -> Result<Self, AsyncEnvelopeError> {
        let verified = authorized.verified().clone();
        let validated = validate_membership(&verified, &subscription, registry)?;
        Ok(Self {
            verified,
            subscription,
            stream: validated.stream,
            authoritative_baseline: authorized.verified().baseline(),
            events: validated.events,
            presentation_signals: validated.presentation_signals,
        })
    }

    /// Returns the active logical subscription scope.
    #[must_use]
    pub const fn subscription(&self) -> &SubscriptionId {
        &self.subscription
    }

    /// Returns the active registered stream scope.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns the signed Task 2 descriptor baseline for this exact scope.
    #[must_use]
    pub const fn authoritative_baseline(&self) -> StreamPosition {
        self.authoritative_baseline
    }

    /// Checks immutable membership and registered-payload facts without host callbacks.
    pub(crate) fn validate_local_envelope(
        &self,
        envelope: &AsyncEnvelope,
    ) -> Result<(), AsyncEnvelopeError> {
        if envelope.subscription != self.subscription {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::SubscriptionMismatch,
            ));
        }
        if envelope.stream != self.stream {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::StreamMismatch,
            ));
        }
        validate_registered_payload(self, &envelope.payload)
    }

    /// Freshly admits one envelope against current host membership and registry authority.
    ///
    /// The returned guard is non-cloneable and consumed by the sequence machine.
    /// Physical transport membership remains outside this operation.
    #[cfg(test)]
    pub(crate) fn admit<'a>(
        &'a self,
        envelope: &'a AsyncEnvelope,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
    ) -> Result<ActiveAsyncMembershipGuard<'a>, AsyncEnvelopeError> {
        if now >= self.verified.expires_at() {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::MembershipExpired,
            ));
        }
        if envelope.subscription != self.subscription {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::SubscriptionMismatch,
            ));
        }
        if envelope.stream != self.stream {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::StreamMismatch,
            ));
        }
        let validated = validate_membership(&self.verified, &self.subscription, registry)?;
        if validated.stream != self.stream
            || validated.events != self.events
            || validated.presentation_signals != self.presentation_signals
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        validate_registered_payload(self, &envelope.payload)?;
        Ok(ActiveAsyncMembershipGuard {
            context: self,
            envelope,
        })
    }

    pub(crate) fn admit_owned(
        &self,
        envelope: AsyncEnvelope,
        binding: &SubscriptionBinding,
        document_scope: &DocumentAuthorizationScope,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
    ) -> Result<OwnedActiveAsyncMembershipGuard, AsyncEnvelopeError> {
        if now >= self.verified.expires_at() {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::MembershipExpired,
            ));
        }
        if envelope.subscription != self.subscription {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::SubscriptionMismatch,
            ));
        }
        if envelope.stream != self.stream {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::StreamMismatch,
            ));
        }
        let validated = validate_delivery_membership(
            &self.verified,
            &self.subscription,
            &envelope,
            binding,
            document_scope,
            registry,
        )?;
        if validated.stream != self.stream
            || validated.events != self.events
            || validated.presentation_signals != self.presentation_signals
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        validate_registered_payload(self, &envelope.payload)?;
        Ok(OwnedActiveAsyncMembershipGuard {
            context: self.clone(),
            envelope,
            resolved_event: validated.resolved_event,
        })
    }

    pub(crate) fn admit_replay_owned(
        &self,
        envelopes: Vec<AsyncEnvelope>,
        binding: &SubscriptionBinding,
        document_scope: &DocumentAuthorizationScope,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
    ) -> Result<Vec<OwnedActiveAsyncMembershipGuard>, AsyncEnvelopeError> {
        if now >= self.verified.expires_at() {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::MembershipExpired,
            ));
        }
        for envelope in &envelopes {
            if envelope.subscription != self.subscription {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::SubscriptionMismatch,
                ));
            }
            if envelope.stream != self.stream {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::StreamMismatch,
                ));
            }
            validate_registered_payload(self, &envelope.payload)?;
        }
        let validated = validate_replay_membership(
            &self.verified,
            &self.subscription,
            &envelopes,
            binding,
            document_scope,
            registry,
        )?;
        if validated.stream != self.stream
            || validated.events != self.events
            || validated.presentation_signals != self.presentation_signals
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        Ok(envelopes
            .into_iter()
            .zip(validated.resolved_events)
            .map(
                |(envelope, resolved_event)| OwnedActiveAsyncMembershipGuard {
                    context: self.clone(),
                    envelope,
                    resolved_event,
                },
            )
            .collect())
    }

    pub(crate) fn validate_current_scope(
        &self,
        binding: &SubscriptionBinding,
        document_scope: &DocumentAuthorizationScope,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
    ) -> Result<(), AsyncEnvelopeError> {
        if now >= self.verified.expires_at() {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::MembershipExpired,
            ));
        }
        let validated = validate_delivery_scope(
            &self.verified,
            &self.subscription,
            binding,
            document_scope,
            registry,
        )?;
        if validated.stream != self.stream
            || validated.events != self.events
            || validated.presentation_signals != self.presentation_signals
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ));
        }
        Ok(())
    }
}

fn validate_membership(
    verified: &VerifiedSubscriptionDescriptor,
    subscription: &SubscriptionId,
    registry: &dyn AsyncMembershipRegistryPort,
) -> Result<ValidatedMembership, AsyncEnvelopeError> {
    let request = AsyncMembershipRequest {
        verified,
        subscription,
        envelope: None,
        binding: None,
        document_scope: None,
    };
    let mut validation = AsyncMembershipValidation {
        verified,
        expected_envelope: None,
        expected_document_scope: None,
        candidate: None,
        rejected: None,
    };
    registry.validate_current(request, &mut validation);
    validation.finish()
}

fn validate_delivery_membership(
    verified: &VerifiedSubscriptionDescriptor,
    subscription: &SubscriptionId,
    envelope: &AsyncEnvelope,
    binding: &SubscriptionBinding,
    document_scope: &DocumentAuthorizationScope,
    registry: &dyn AsyncMembershipRegistryPort,
) -> Result<ValidatedMembership, AsyncEnvelopeError> {
    let request = AsyncMembershipRequest {
        verified,
        subscription,
        envelope: Some(envelope),
        binding: Some(binding),
        document_scope: Some(document_scope),
    };
    let mut validation = AsyncMembershipValidation {
        verified,
        expected_envelope: Some(envelope),
        expected_document_scope: Some(document_scope),
        candidate: None,
        rejected: None,
    };
    registry.validate_current(request, &mut validation);
    validation.finish()
}

fn validate_replay_membership(
    verified: &VerifiedSubscriptionDescriptor,
    subscription: &SubscriptionId,
    envelopes: &[AsyncEnvelope],
    binding: &SubscriptionBinding,
    document_scope: &DocumentAuthorizationScope,
    registry: &dyn AsyncMembershipRegistryPort,
) -> Result<ValidatedReplayMembership, AsyncEnvelopeError> {
    let request = AsyncReplayMembershipRequest {
        verified,
        subscription,
        envelopes,
        binding,
        document_scope,
    };
    let mut validation = AsyncReplayMembershipValidation {
        verified,
        expected_envelopes: envelopes,
        expected_document_scope: document_scope,
        candidate: None,
        rejected: None,
    };
    registry.validate_replay_current(request, &mut validation);
    validation.finish()
}

fn validate_delivery_scope(
    verified: &VerifiedSubscriptionDescriptor,
    subscription: &SubscriptionId,
    binding: &SubscriptionBinding,
    document_scope: &DocumentAuthorizationScope,
    registry: &dyn AsyncMembershipRegistryPort,
) -> Result<ValidatedMembership, AsyncEnvelopeError> {
    let request = AsyncMembershipRequest {
        verified,
        subscription,
        envelope: None,
        binding: Some(binding),
        document_scope: Some(document_scope),
    };
    let mut validation = AsyncMembershipValidation {
        verified,
        expected_envelope: None,
        expected_document_scope: Some(document_scope),
        candidate: None,
        rejected: None,
    };
    registry.validate_current(request, &mut validation);
    validation.finish()
}

/// One-use proof that an envelope passed current host membership admission.
///
/// ```compile_fail
/// use suprnova_live::async_updates::{
///     ActiveAsyncMembershipGuard, AsyncEnvelopeDispatchPort, SequenceMachine,
/// };
/// use suprnova_live::identity::UnixMillis;
///
/// fn cannot_reuse(
///     guard: ActiveAsyncMembershipGuard<'_>,
///     machine: &mut SequenceMachine,
///     dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
/// ) {
///     let _ = machine.dispatch(guard, UnixMillis::new(0), dispatcher);
///     let _ = machine.dispatch(guard, UnixMillis::new(0), dispatcher);
/// }
/// ```
pub(crate) struct ActiveAsyncMembershipGuard<'a> {
    context: &'a AsyncEnvelopeContext,
    envelope: &'a AsyncEnvelope,
}

impl<'a> ActiveAsyncMembershipGuard<'a> {
    pub(crate) fn is_current_at(&self, now: UnixMillis) -> bool {
        now < self.context.verified.expires_at()
    }

    pub(crate) const fn context(&self) -> &'a AsyncEnvelopeContext {
        self.context
    }

    pub(crate) const fn envelope(&self) -> &'a AsyncEnvelope {
        self.envelope
    }
}

impl fmt::Debug for ActiveAsyncMembershipGuard<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ActiveAsyncMembershipGuard:redacted>")
    }
}

/// Owned one-use proof retained across bounded document delivery.
pub(crate) struct OwnedActiveAsyncMembershipGuard {
    context: AsyncEnvelopeContext,
    envelope: AsyncEnvelope,
    resolved_event: Option<ResolvedEventFanout>,
}

impl OwnedActiveAsyncMembershipGuard {
    pub(crate) const fn envelope(&self) -> &AsyncEnvelope {
        &self.envelope
    }

    pub(crate) const fn resolved_event(&self) -> Option<&ResolvedEventFanout> {
        self.resolved_event.as_ref()
    }

    pub(crate) const fn as_active(&self) -> ActiveAsyncMembershipGuard<'_> {
        ActiveAsyncMembershipGuard {
            context: &self.context,
            envelope: &self.envelope,
        }
    }
}

impl fmt::Debug for OwnedActiveAsyncMembershipGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<OwnedActiveAsyncMembershipGuard:redacted>")
    }
}

/// Closed fresh-render operation registered by the async protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisteredRefresh;

/// Schema- and target-validated browser event from current subscription metadata.
#[derive(Clone, PartialEq)]
pub struct RegisteredBrowserEvent {
    name: BrowserOperationName,
    schema_version: u16,
    target: EventTarget,
    payload: CanonicalValue,
    maximum_fanout: NonZeroU16,
}

impl fmt::Debug for RegisteredBrowserEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredBrowserEvent")
            .field("name", &self.name)
            .field("schema_version", &self.schema_version)
            .field("target", &self.target)
            .field("payload", &"<redacted>")
            .finish()
    }
}

impl RegisteredBrowserEvent {
    /// Creates a server-authored event only when it matches current registration.
    pub fn new(
        context: &AsyncEnvelopeContext,
        name: BrowserOperationName,
        schema_version: u16,
        target: EventTarget,
        payload: CanonicalValue,
    ) -> Result<Self, AsyncEnvelopeError> {
        let maximum_fanout = registered_event_contract(context, &name)?.maximum_fanout();
        let event = Self {
            name,
            schema_version,
            target,
            payload,
            maximum_fanout,
        };
        match validate_programmatic_payload(context, &AsyncPayload::BrowserEvent(event))? {
            AsyncPayload::BrowserEvent(event) => Ok(event),
            _ => unreachable!("validated browser-event payload preserves its closed variant"),
        }
    }

    /// Returns the registered event identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the registered event schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns the registered propagation target selected by this event.
    #[must_use]
    pub const fn target(&self) -> &EventTarget {
        &self.target
    }

    /// Returns the bounded structured payload.
    #[must_use]
    pub const fn payload(&self) -> &CanonicalValue {
        &self.payload
    }

    /// Returns the current descriptor-bound delivery fanout ceiling.
    #[must_use]
    pub const fn maximum_fanout(&self) -> NonZeroU16 {
        self.maximum_fanout
    }
}

/// Declared presentation-only local-signal update.
#[derive(Clone, PartialEq)]
pub struct RegisteredPresentationSignal {
    name: BrowserOperationName,
    value: CanonicalValue,
    schema: BrowserPayloadSchema,
}

impl fmt::Debug for RegisteredPresentationSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredPresentationSignal")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

impl RegisteredPresentationSignal {
    /// Creates a server-authored signal only when it matches current registration.
    pub fn new(
        context: &AsyncEnvelopeContext,
        name: BrowserOperationName,
        value: CanonicalValue,
    ) -> Result<Self, AsyncEnvelopeError> {
        let schema = registered_presentation_signal_contract(context, &name)?.schema();
        let signal = Self {
            name,
            value,
            schema,
        };
        match validate_programmatic_payload(context, &AsyncPayload::PresentationSignal(signal))? {
            AsyncPayload::PresentationSignal(signal) => Ok(signal),
            _ => unreachable!("validated presentation-signal payload preserves its closed variant"),
        }
    }

    /// Returns the declared local signal identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the schema-validated presentation value.
    #[must_use]
    pub const fn value(&self) -> &CanonicalValue {
        &self.value
    }

    /// Returns the current registered payload schema used for coalescing identity.
    #[must_use]
    pub const fn schema(&self) -> BrowserPayloadSchema {
        self.schema
    }
}

/// Transport heartbeat carrying sequence continuity but no productive authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Heartbeat;

/// Closed completion disposition for one logical stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionReason {
    /// The host is performing a controlled shutdown.
    ServerShutdown,
    /// Current subscription authority was deliberately retired.
    SubscriptionRetired,
    /// The registered stream completed normally.
    StreamCompleted,
}

/// Closed safe error code for asynchronous stream state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamErrorCode {
    /// Current authorization was lost or revoked.
    AuthorizationLost,
    /// Requested continuity replay is no longer available.
    ReplayUnavailable,
    /// Server backpressure requires recovery before more application.
    Backpressure,
    /// The host could not continue the registered stream.
    StreamUnavailable,
}

/// Closed productive and lifecycle payload union for async protocol v1.
#[derive(Clone, Debug, PartialEq)]
pub enum AsyncPayload {
    /// Queue one registered fresh render through the ordinary scheduler.
    Refresh(RegisteredRefresh),
    /// Dispatch one current registered typed browser event.
    BrowserEvent(RegisteredBrowserEvent),
    /// Update one declared presentation-only local signal.
    PresentationSignal(RegisteredPresentationSignal),
    /// Observe transport liveness and sequence continuity.
    Heartbeat(Heartbeat),
    /// Close the logical stream with a bounded reason.
    Complete(CompletionReason),
    /// Degrade the logical stream with a bounded safe code.
    Error(StreamErrorCode),
}

/// Fully validated async event envelope bound to one active membership.
#[derive(Clone, Debug, PartialEq)]
pub struct AsyncEnvelope {
    protocol_version: u16,
    subscription: SubscriptionId,
    stream: StreamName,
    position: StreamPosition,
    payload: AsyncPayload,
}

impl AsyncEnvelope {
    /// Creates one server-authored v1 envelope bound to current membership.
    pub fn new(
        context: &AsyncEnvelopeContext,
        position: StreamPosition,
        payload: AsyncPayload,
    ) -> Result<Self, AsyncEnvelopeError> {
        let payload = validate_programmatic_payload(context, &payload)?;
        let envelope = Self {
            protocol_version: 1,
            subscription: context.subscription.clone(),
            stream: context.stream.clone(),
            position,
            payload,
        };
        let limits = AsyncCodecLimits::v1();
        let encoded = encode_async_envelope(&envelope, &limits)?;
        decode_async_envelope(&encoded, &limits, context)
    }

    /// Returns the independently versioned async protocol major.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    /// Returns the validated active logical subscription identity.
    #[must_use]
    pub const fn subscription(&self) -> &SubscriptionId {
        &self.subscription
    }

    /// Returns the registered stream identity.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns the server-authored stream position.
    #[must_use]
    pub const fn position(&self) -> StreamPosition {
        self.position
    }

    /// Returns the closed registered payload.
    #[must_use]
    pub const fn payload(&self) -> &AsyncPayload {
        &self.payload
    }
}

/// Decodes one canonical bounded envelope after selecting its active membership context.
pub fn decode_async_envelope(
    encoded: &[u8],
    limits: &AsyncCodecLimits,
    context: &AsyncEnvelopeContext,
) -> Result<AsyncEnvelope, AsyncEnvelopeError> {
    if encoded.len() > limits.input().max_bytes() {
        return Err(AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::TooLarge));
    }
    preflight_payload_size(encoded, limits)?;
    let canonical = parse_canonical_value(encoded, &limits.input()).map_err(map_canonical)?;
    let recoded = to_canonical_bytes(&canonical, &limits.input()).map_err(map_canonical)?;
    if recoded != encoded {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::NonCanonical,
        ));
    }

    let mut fields = object(canonical, AsyncEnvelopeErrorKind::InvalidEnvelope)?;
    require_exact_keys(
        &fields,
        &ENVELOPE_KEYS,
        AsyncEnvelopeErrorKind::InvalidEnvelope,
    )?;
    let protocol_version = unsigned(
        take(&mut fields, "protocol_version")?,
        AsyncEnvelopeErrorKind::InvalidEnvelope,
    )?;
    let protocol_version = u16::try_from(protocol_version)
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnsupportedProtocol))?;
    if !SUPPORTED_ASYNC_PROTOCOL_VERSIONS.contains(&protocol_version) {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnsupportedProtocol,
        ));
    }

    let subscription = SubscriptionId::parse(&string(take(&mut fields, "subscription")?)?)?;
    if subscription != context.subscription {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::SubscriptionMismatch,
        ));
    }
    let stream = StreamName::parse(&string(take(&mut fields, "stream")?)?)
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::InvalidEnvelope))?;
    if stream != context.stream {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::StreamMismatch,
        ));
    }
    let position = parse_position(take(&mut fields, "position")?)?;
    let payload_value = take(&mut fields, "payload")?;
    let payload_bytes =
        to_canonical_bytes(&payload_value, &limits.input()).map_err(map_canonical)?;
    if payload_bytes.len() > limits.max_payload_bytes() {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::PayloadTooLarge,
        ));
    }
    let payload = parse_payload(payload_value, context)?;

    Ok(AsyncEnvelope {
        protocol_version,
        subscription,
        stream,
        position,
        payload,
    })
}

fn preflight_payload_size(
    encoded: &[u8],
    limits: &AsyncCodecLimits,
) -> Result<(), AsyncEnvelopeError> {
    let mut cursor = 0;
    skip_json_whitespace(encoded, &mut cursor);
    if encoded.get(cursor) != Some(&b'{') {
        return Ok(());
    }
    cursor += 1;
    loop {
        skip_json_whitespace(encoded, &mut cursor);
        if encoded.get(cursor) == Some(&b'}') {
            return Ok(());
        }
        let (key_start, key_end, escaped) = scan_json_string(encoded, &mut cursor)?;
        if escaped {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::NonCanonical,
            ));
        }
        skip_json_whitespace(encoded, &mut cursor);
        if encoded.get(cursor) != Some(&b':') {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::InvalidEnvelope,
            ));
        }
        cursor += 1;
        skip_json_whitespace(encoded, &mut cursor);
        let value_start = cursor;
        scan_json_value(encoded, &mut cursor, 0, limits.input().max_depth())?;
        if &encoded[key_start..key_end] == b"payload"
            && cursor.saturating_sub(value_start) > limits.max_payload_bytes()
        {
            return Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::PayloadTooLarge,
            ));
        }
        skip_json_whitespace(encoded, &mut cursor);
        match encoded.get(cursor) {
            Some(b',') => cursor += 1,
            Some(b'}') => return Ok(()),
            _ => {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::InvalidEnvelope,
                ));
            }
        }
    }
}

fn scan_json_value(
    encoded: &[u8],
    cursor: &mut usize,
    depth: usize,
    max_depth: usize,
) -> Result<(), AsyncEnvelopeError> {
    let Some(byte) = encoded.get(*cursor).copied() else {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        ));
    };
    match byte {
        b'"' => {
            scan_json_string(encoded, cursor)?;
        }
        b'{' => {
            if depth >= max_depth {
                return Err(AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::TooDeep));
            }
            *cursor += 1;
            loop {
                skip_json_whitespace(encoded, cursor);
                if encoded.get(*cursor) == Some(&b'}') {
                    *cursor += 1;
                    break;
                }
                scan_json_string(encoded, cursor)?;
                skip_json_whitespace(encoded, cursor);
                if encoded.get(*cursor) != Some(&b':') {
                    return Err(AsyncEnvelopeError::new(
                        AsyncEnvelopeErrorKind::InvalidEnvelope,
                    ));
                }
                *cursor += 1;
                skip_json_whitespace(encoded, cursor);
                scan_json_value(encoded, cursor, depth + 1, max_depth)?;
                skip_json_whitespace(encoded, cursor);
                match encoded.get(*cursor) {
                    Some(b',') => *cursor += 1,
                    Some(b'}') => {
                        *cursor += 1;
                        break;
                    }
                    _ => {
                        return Err(AsyncEnvelopeError::new(
                            AsyncEnvelopeErrorKind::InvalidEnvelope,
                        ));
                    }
                }
            }
        }
        b'[' => {
            if depth >= max_depth {
                return Err(AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::TooDeep));
            }
            *cursor += 1;
            loop {
                skip_json_whitespace(encoded, cursor);
                if encoded.get(*cursor) == Some(&b']') {
                    *cursor += 1;
                    break;
                }
                scan_json_value(encoded, cursor, depth + 1, max_depth)?;
                skip_json_whitespace(encoded, cursor);
                match encoded.get(*cursor) {
                    Some(b',') => *cursor += 1,
                    Some(b']') => {
                        *cursor += 1;
                        break;
                    }
                    _ => {
                        return Err(AsyncEnvelopeError::new(
                            AsyncEnvelopeErrorKind::InvalidEnvelope,
                        ));
                    }
                }
            }
        }
        _ => {
            let start = *cursor;
            while encoded.get(*cursor).is_some_and(|byte| {
                !matches!(byte, b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t')
            }) {
                *cursor += 1;
            }
            if *cursor == start {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::InvalidEnvelope,
                ));
            }
        }
    }
    Ok(())
}

fn scan_json_string(
    encoded: &[u8],
    cursor: &mut usize,
) -> Result<(usize, usize, bool), AsyncEnvelopeError> {
    if encoded.get(*cursor) != Some(&b'"') {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        ));
    }
    *cursor += 1;
    let start = *cursor;
    let mut escaped = false;
    loop {
        match encoded.get(*cursor).copied() {
            Some(b'"') => {
                let end = *cursor;
                *cursor += 1;
                return Ok((start, end, escaped));
            }
            Some(b'\\') => {
                escaped = true;
                *cursor = (*cursor).saturating_add(2);
                if *cursor > encoded.len() {
                    return Err(AsyncEnvelopeError::new(
                        AsyncEnvelopeErrorKind::InvalidEnvelope,
                    ));
                }
            }
            Some(_) => *cursor += 1,
            None => {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::InvalidEnvelope,
                ));
            }
        }
    }
}

fn skip_json_whitespace(encoded: &[u8], cursor: &mut usize) {
    while encoded
        .get(*cursor)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        *cursor += 1;
    }
}

/// Encodes one already validated async envelope into its stable canonical wire form.
pub fn encode_async_envelope(
    envelope: &AsyncEnvelope,
    limits: &AsyncCodecLimits,
) -> Result<Vec<u8>, AsyncEnvelopeError> {
    let payload = payload_value(&envelope.payload)?;
    let payload_bytes = to_canonical_bytes(&payload, &limits.input()).map_err(map_canonical)?;
    if payload_bytes.len() > limits.max_payload_bytes() {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::PayloadTooLarge,
        ));
    }
    let position = CanonicalValue::Object(BTreeMap::from([
        (
            "epoch".to_owned(),
            CanonicalValue::String(envelope.position.epoch().get().to_string()),
        ),
        (
            "sequence".to_owned(),
            CanonicalValue::String(envelope.position.sequence().get().to_string()),
        ),
    ]));
    let value = CanonicalValue::Object(BTreeMap::from([
        ("payload".to_owned(), payload),
        ("position".to_owned(), position),
        (
            "protocol_version".to_owned(),
            CanonicalValue::number(f64::from(envelope.protocol_version)).map_err(map_canonical)?,
        ),
        (
            "stream".to_owned(),
            CanonicalValue::String(envelope.stream.as_str().to_owned()),
        ),
        (
            "subscription".to_owned(),
            CanonicalValue::String(envelope.subscription.to_base64url()),
        ),
    ]));
    to_canonical_bytes(&value, &limits.input()).map_err(map_canonical)
}

pub(crate) fn canonical_async_payload_len(
    envelope: &AsyncEnvelope,
) -> Result<usize, AsyncEnvelopeError> {
    let limits = AsyncCodecLimits::v1();
    let payload = payload_value(envelope.payload())?;
    to_canonical_bytes(&payload, &limits.input())
        .map(|bytes| bytes.len())
        .map_err(map_canonical)
}

fn parse_position(value: CanonicalValue) -> Result<StreamPosition, AsyncEnvelopeError> {
    let mut fields = object(value, AsyncEnvelopeErrorKind::InvalidEnvelope)?;
    require_exact_keys(
        &fields,
        &POSITION_KEYS,
        AsyncEnvelopeErrorKind::InvalidEnvelope,
    )?;
    let epoch = decimal(&string(take(&mut fields, "epoch")?)?)?;
    let sequence = decimal(&string(take(&mut fields, "sequence")?)?)?;
    Ok(StreamPosition::new(
        StreamEpoch::new(epoch),
        StreamSequence::new(sequence),
    ))
}

fn parse_payload(
    value: CanonicalValue,
    context: &AsyncEnvelopeContext,
) -> Result<AsyncPayload, AsyncEnvelopeError> {
    let mut fields = object(value, AsyncEnvelopeErrorKind::InvalidPayload)?;
    let kind = fields
        .get("kind")
        .cloned()
        .ok_or_else(|| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::InvalidPayload))
        .and_then(string)?;
    match kind.as_str() {
        "refresh" => {
            require_exact_keys(
                &fields,
                &["kind", "name"],
                AsyncEnvelopeErrorKind::InvalidPayload,
            )?;
            if string(take(&mut fields, "name")?)? != "refresh" {
                return Err(AsyncEnvelopeError::new(
                    AsyncEnvelopeErrorKind::InvalidPayload,
                ));
            }
            Ok(AsyncPayload::Refresh(RegisteredRefresh))
        }
        "browser_event" => parse_browser_event(fields, context).map(AsyncPayload::BrowserEvent),
        "presentation_signal" => {
            parse_presentation_signal(fields, context).map(AsyncPayload::PresentationSignal)
        }
        "heartbeat" => {
            require_exact_keys(&fields, &["kind"], AsyncEnvelopeErrorKind::InvalidPayload)?;
            Ok(AsyncPayload::Heartbeat(Heartbeat))
        }
        "complete" => {
            require_exact_keys(
                &fields,
                &["kind", "reason"],
                AsyncEnvelopeErrorKind::InvalidPayload,
            )?;
            let reason = match string(take(&mut fields, "reason")?)?.as_str() {
                "server_shutdown" => CompletionReason::ServerShutdown,
                "subscription_retired" => CompletionReason::SubscriptionRetired,
                "stream_completed" => CompletionReason::StreamCompleted,
                _ => {
                    return Err(AsyncEnvelopeError::new(
                        AsyncEnvelopeErrorKind::InvalidPayload,
                    ));
                }
            };
            Ok(AsyncPayload::Complete(reason))
        }
        "error" => {
            require_exact_keys(
                &fields,
                &["code", "kind"],
                AsyncEnvelopeErrorKind::InvalidPayload,
            )?;
            let code = match string(take(&mut fields, "code")?)?.as_str() {
                "authorization_lost" => StreamErrorCode::AuthorizationLost,
                "replay_unavailable" => StreamErrorCode::ReplayUnavailable,
                "backpressure" => StreamErrorCode::Backpressure,
                "stream_unavailable" => StreamErrorCode::StreamUnavailable,
                _ => {
                    return Err(AsyncEnvelopeError::new(
                        AsyncEnvelopeErrorKind::InvalidPayload,
                    ));
                }
            };
            Ok(AsyncPayload::Error(code))
        }
        _ => Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnsupportedPayload,
        )),
    }
}

fn parse_browser_event(
    mut fields: BTreeMap<String, CanonicalValue>,
    context: &AsyncEnvelopeContext,
) -> Result<RegisteredBrowserEvent, AsyncEnvelopeError> {
    require_exact_keys(
        &fields,
        &["event", "kind", "payload", "schema_version", "target"],
        AsyncEnvelopeErrorKind::InvalidPayload,
    )?;
    let name = BrowserOperationName::parse(&string(take(&mut fields, "event")?)?)
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload))?;
    let schema_version = unsigned(
        take(&mut fields, "schema_version")?,
        AsyncEnvelopeErrorKind::InvalidPayload,
    )?;
    let schema_version = u16::try_from(schema_version)
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload))?;
    let target = parse_target(&string(take(&mut fields, "target")?)?)?;
    let payload = take(&mut fields, "payload")?;
    let contract = registered_event_contract(context, &name)?;
    validate_event(contract, schema_version, &target, &payload)?;
    Ok(RegisteredBrowserEvent {
        name,
        schema_version,
        target,
        payload,
        maximum_fanout: contract.maximum_fanout(),
    })
}

fn validate_event(
    contract: &SubscriptionEventContract,
    schema_version: u16,
    target: &EventTarget,
    payload: &CanonicalValue,
) -> Result<(), AsyncEnvelopeError> {
    if contract.version() != schema_version
        || !contract.targets().as_slice().contains(target)
        || !schema_matches(contract.schema(), payload)
    {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        ));
    }
    Ok(())
}

fn validate_registered_event(
    context: &AsyncEnvelopeContext,
    event: &RegisteredBrowserEvent,
) -> Result<(), AsyncEnvelopeError> {
    let contract = registered_event_contract(context, event.name())?;
    if event.maximum_fanout() != contract.maximum_fanout() {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        ));
    }
    validate_event(
        contract,
        event.schema_version(),
        event.target(),
        event.payload(),
    )
}

fn registered_event_contract<'a>(
    context: &'a AsyncEnvelopeContext,
    name: &BrowserOperationName,
) -> Result<&'a SubscriptionEventContract, AsyncEnvelopeError> {
    context
        .events
        .as_slice()
        .iter()
        .find(|contract| contract.name() == name)
        .ok_or_else(|| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload))
}

fn validate_registered_signal(
    context: &AsyncEnvelopeContext,
    signal: &RegisteredPresentationSignal,
) -> Result<(), AsyncEnvelopeError> {
    let contract = registered_presentation_signal_contract(context, signal.name())?;
    if signal.schema() != contract.schema() || !schema_matches(contract.schema(), signal.value()) {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        ));
    }
    Ok(())
}

fn registered_presentation_signal_contract<'a>(
    context: &'a AsyncEnvelopeContext,
    name: &BrowserOperationName,
) -> Result<&'a PresentationSignalContract, AsyncEnvelopeError> {
    context
        .presentation_signals
        .find(name)
        .ok_or_else(|| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload))
}

fn validate_registered_payload(
    context: &AsyncEnvelopeContext,
    payload: &AsyncPayload,
) -> Result<(), AsyncEnvelopeError> {
    match payload {
        AsyncPayload::BrowserEvent(event) => validate_registered_event(context, event),
        AsyncPayload::PresentationSignal(signal) => validate_registered_signal(context, signal),
        AsyncPayload::Refresh(_)
        | AsyncPayload::Heartbeat(_)
        | AsyncPayload::Complete(_)
        | AsyncPayload::Error(_) => Ok(()),
    }
}

fn validate_programmatic_payload(
    context: &AsyncEnvelopeContext,
    payload: &AsyncPayload,
) -> Result<AsyncPayload, AsyncEnvelopeError> {
    validate_registered_payload(context, payload)?;
    let limits = AsyncCodecLimits::v1();
    let value = payload_value(payload)?;
    let encoded = to_canonical_bytes(&value, &limits.input()).map_err(map_canonical)?;
    if encoded.len() > limits.max_payload_bytes() {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::PayloadTooLarge,
        ));
    }
    let reparsed = parse_canonical_value(&encoded, &limits.input()).map_err(map_canonical)?;
    let recoded = to_canonical_bytes(&reparsed, &limits.input()).map_err(map_canonical)?;
    if recoded != encoded {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::NonCanonical,
        ));
    }
    parse_payload(reparsed, context)
}

fn parse_presentation_signal(
    mut fields: BTreeMap<String, CanonicalValue>,
    context: &AsyncEnvelopeContext,
) -> Result<RegisteredPresentationSignal, AsyncEnvelopeError> {
    require_exact_keys(
        &fields,
        &["kind", "name", "value"],
        AsyncEnvelopeErrorKind::InvalidPayload,
    )?;
    let name = BrowserOperationName::parse(&string(take(&mut fields, "name")?)?)
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload))?;
    let value = take(&mut fields, "value")?;
    let contract = registered_presentation_signal_contract(context, &name)?;
    if !schema_matches(contract.schema(), &value) {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        ));
    }
    Ok(RegisteredPresentationSignal {
        name,
        value,
        schema: contract.schema(),
    })
}

fn parse_target(value: &str) -> Result<EventTarget, AsyncEnvelopeError> {
    match value {
        "self" => Ok(EventTarget::SelfIsland),
        "parent" => Ok(EventTarget::Parent),
        "child" => Ok(EventTarget::Child),
        "document" => Ok(EventTarget::Document),
        _ => {
            if let Some(slot) = value.strip_prefix("named_island:") {
                return IslandSlot::parse(slot)
                    .map(EventTarget::NamedIsland)
                    .map_err(|_| {
                        AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload)
                    });
            }
            if let Some(listener) = value.strip_prefix("browser:") {
                return BrowserOperationName::parse(listener)
                    .map(EventTarget::Browser)
                    .map_err(|_| {
                        AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::UnregisteredPayload)
                    });
            }
            Err(AsyncEnvelopeError::new(
                AsyncEnvelopeErrorKind::UnregisteredPayload,
            ))
        }
    }
}

fn schema_matches(schema: BrowserPayloadSchema, value: &CanonicalValue) -> bool {
    match (schema, value) {
        (BrowserPayloadSchema::Json, _) | (BrowserPayloadSchema::Null, CanonicalValue::Null) => {
            true
        }
        (BrowserPayloadSchema::Boolean, CanonicalValue::Bool(_))
        | (BrowserPayloadSchema::String, CanonicalValue::String(_)) => true,
        (BrowserPayloadSchema::I64, CanonicalValue::Number(number)) => {
            let value = number.get();
            value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64
        }
        (BrowserPayloadSchema::U64, CanonicalValue::Number(number)) => {
            let value = number.get();
            value.fract() == 0.0 && value >= 0.0 && value <= MAX_SAFE_INTEGER as f64
        }
        (BrowserPayloadSchema::F64, CanonicalValue::Number(_)) => true,
        _ => false,
    }
}

fn payload_value(payload: &AsyncPayload) -> Result<CanonicalValue, AsyncEnvelopeError> {
    let value = match payload {
        AsyncPayload::Refresh(_) => object_value([
            ("kind", CanonicalValue::String("refresh".to_owned())),
            ("name", CanonicalValue::String("refresh".to_owned())),
        ]),
        AsyncPayload::BrowserEvent(event) => object_value([
            (
                "event",
                CanonicalValue::String(event.name.as_str().to_owned()),
            ),
            ("kind", CanonicalValue::String("browser_event".to_owned())),
            ("payload", event.payload.clone()),
            (
                "schema_version",
                CanonicalValue::number(f64::from(event.schema_version)).map_err(map_canonical)?,
            ),
            ("target", CanonicalValue::String(target_name(&event.target))),
        ]),
        AsyncPayload::PresentationSignal(signal) => object_value([
            (
                "kind",
                CanonicalValue::String("presentation_signal".to_owned()),
            ),
            (
                "name",
                CanonicalValue::String(signal.name.as_str().to_owned()),
            ),
            ("value", signal.value.clone()),
        ]),
        AsyncPayload::Heartbeat(_) => {
            object_value([("kind", CanonicalValue::String("heartbeat".to_owned()))])
        }
        AsyncPayload::Complete(reason) => object_value([
            ("kind", CanonicalValue::String("complete".to_owned())),
            (
                "reason",
                CanonicalValue::String(
                    match reason {
                        CompletionReason::ServerShutdown => "server_shutdown",
                        CompletionReason::SubscriptionRetired => "subscription_retired",
                        CompletionReason::StreamCompleted => "stream_completed",
                    }
                    .to_owned(),
                ),
            ),
        ]),
        AsyncPayload::Error(code) => object_value([
            (
                "code",
                CanonicalValue::String(
                    match code {
                        StreamErrorCode::AuthorizationLost => "authorization_lost",
                        StreamErrorCode::ReplayUnavailable => "replay_unavailable",
                        StreamErrorCode::Backpressure => "backpressure",
                        StreamErrorCode::StreamUnavailable => "stream_unavailable",
                    }
                    .to_owned(),
                ),
            ),
            ("kind", CanonicalValue::String("error".to_owned())),
        ]),
    };
    Ok(value)
}

fn target_name(target: &EventTarget) -> String {
    match target {
        EventTarget::SelfIsland => "self".to_owned(),
        EventTarget::Parent => "parent".to_owned(),
        EventTarget::Child => "child".to_owned(),
        EventTarget::NamedIsland(slot) => format!("named_island:{}", slot.as_str()),
        EventTarget::Document => "document".to_owned(),
        EventTarget::Browser(listener) => format!("browser:{}", listener.as_str()),
    }
}

fn object_value<const N: usize>(entries: [(&str, CanonicalValue); N]) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn object(
    value: CanonicalValue,
    kind: AsyncEnvelopeErrorKind,
) -> Result<BTreeMap<String, CanonicalValue>, AsyncEnvelopeError> {
    match value {
        CanonicalValue::Object(fields) => Ok(fields),
        _ => Err(AsyncEnvelopeError::new(kind)),
    }
}

fn require_exact_keys(
    fields: &BTreeMap<String, CanonicalValue>,
    expected: &[&str],
    kind: AsyncEnvelopeErrorKind,
) -> Result<(), AsyncEnvelopeError> {
    if fields.len() != expected.len() || expected.iter().any(|key| !fields.contains_key(*key)) {
        return Err(AsyncEnvelopeError::new(kind));
    }
    Ok(())
}

fn take(
    fields: &mut BTreeMap<String, CanonicalValue>,
    key: &str,
) -> Result<CanonicalValue, AsyncEnvelopeError> {
    fields
        .remove(key)
        .ok_or_else(|| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::InvalidEnvelope))
}

fn string(value: CanonicalValue) -> Result<String, AsyncEnvelopeError> {
    match value {
        CanonicalValue::String(value) => Ok(value),
        _ => Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        )),
    }
}

fn unsigned(
    value: CanonicalValue,
    kind: AsyncEnvelopeErrorKind,
) -> Result<u64, AsyncEnvelopeError> {
    let CanonicalValue::Number(value) = value else {
        return Err(AsyncEnvelopeError::new(kind));
    };
    let value = value.get();
    if value.fract() != 0.0 || value < 0.0 || value > u64::MAX as f64 {
        return Err(AsyncEnvelopeError::new(kind));
    }
    Ok(value as u64)
}

fn decimal(value: &str) -> Result<u64, AsyncEnvelopeError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(AsyncEnvelopeError::new(
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        ));
    }
    value
        .parse()
        .map_err(|_| AsyncEnvelopeError::new(AsyncEnvelopeErrorKind::InvalidEnvelope))
}

fn map_canonical(error: crate::canonical::CanonicalError) -> AsyncEnvelopeError {
    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge => AsyncEnvelopeErrorKind::TooLarge,
        CanonicalErrorKind::TooDeep => AsyncEnvelopeErrorKind::TooDeep,
        CanonicalErrorKind::TooManyEntries => AsyncEnvelopeErrorKind::TooManyEntries,
        CanonicalErrorKind::StringTooLong => AsyncEnvelopeErrorKind::StringTooLong,
        CanonicalErrorKind::DuplicateKey => AsyncEnvelopeErrorKind::DuplicateField,
        CanonicalErrorKind::InvalidUtf8
        | CanonicalErrorKind::InvalidNumber
        | CanonicalErrorKind::InvalidJson
        | CanonicalErrorKind::SerializationFailed => AsyncEnvelopeErrorKind::InvalidEnvelope,
    };
    AsyncEnvelopeError::new(kind)
}
