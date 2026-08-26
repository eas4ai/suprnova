//! Host-neutral logical subscription and document transport contracts.

use std::error::Error;
use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

use crate::host::HostScopeFacts;
use crate::identity::{ContentDigest, UnixMillis};
use crate::resource::{PermitPool, ResourceBounds, ResourceOwner, Retirement};

use super::backpressure::PressureMembership;
use super::{
    AsyncBackpressure, AsyncBackpressureError, AsyncBufferEntry, AsyncContinuityAuthorityPort,
    AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeDispatchPort, AsyncMembershipRegistryPort,
    AsyncPayload, AsyncPolicy, AsyncTelemetrySnapshot, AuthorizationMemo,
    AuthorizedAsyncBufferEntry, AuthorizedSubscription, BaselineDisposition, BoundedEventContracts,
    BoundedTopics, BufferDisposition, LeaseDispatchError, ReplayDispatchError,
    ReplayDispatchOutcome, ReplayPreflight, SequenceDisposition, SequenceErrorKind,
    SequenceMachine, SequenceState, StreamName, StreamPosition, SubscriptionBinding,
    SubscriptionId, SubscriptionMode, SubscriptionModes, VerifiedSubscriptionDescriptor,
};

const MIN_DOCUMENT_HANDLE_BYTES: usize = 16;
const MAX_DOCUMENT_HANDLE_BYTES: usize = 32;

/// Maximum logical subscriptions retained by one document transport.
pub const MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS: usize = 128;

/// Compact trusted sharing key for one physical document transport.
///
/// The scope binds connection-level host identity and aggregate transport
/// policy only. Component identity and component contract remain part of each
/// logical membership's separate authorization memo.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct DocumentAuthorizationScope(ContentDigest);

impl DocumentAuthorizationScope {
    /// Derives a collision-resistant scope from trusted host facts and policy.
    pub fn derive(
        facts: &HostScopeFacts,
        transport_policy: &ContentDigest,
    ) -> Result<Self, AsyncTransportError> {
        let mut digest = Sha256::new();
        digest.update(b"suprnova-live/document-transport-scope/v1\0");
        hash_document_scope_part(&mut digest, facts.scope().as_bytes());
        hash_optional_document_scope_part(
            &mut digest,
            facts.session().map(|value| value.digest().as_bytes()),
        );
        hash_optional_document_scope_part(
            &mut digest,
            facts.principal().map(|value| value.digest().as_bytes()),
        );
        hash_optional_document_scope_part(
            &mut digest,
            facts.tenant().map(|value| value.digest().as_bytes()),
        );
        hash_document_scope_part(&mut digest, transport_policy.as_bytes());
        let bytes: [u8; 32] = digest.finalize().into();
        ContentDigest::from_bytes(&bytes)
            .map(Self)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))
    }

    /// Returns the canonical non-secret sharing key for trusted host storage.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        self.0.to_base64url()
    }
}

impl fmt::Debug for DocumentAuthorizationScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<DocumentAuthorizationScope:redacted>")
    }
}

/// Executor-neutral future returned by asynchronous transport ports.
pub type AsyncTransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result of accepting one logical or document close transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseDisposition {
    /// This call performed the close transition.
    ///
    /// For document membership removal, this means routing authority was
    /// detached; provider cleanup may remain owned by the retirement lane.
    Closed,
    /// The session had already completed its close transition.
    AlreadyClosed,
}

/// Closed failure vocabulary for logical and physical transport operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncTransportErrorKind {
    /// The supplied origin was absent, malformed, or outside the exact policy.
    InvalidOrigin,
    /// A logical membership belongs to another physical transport origin.
    OriginMismatch,
    /// A membership operation targeted another physical transport kind.
    TransportMismatch,
    /// A logical membership belongs to another authorization scope.
    AuthorizationScopeMismatch,
    /// The logical membership no longer has current authorization.
    AuthorizationLost,
    /// An identical membership already exists.
    DuplicateMembership,
    /// The requested logical membership is not active in this document transport.
    UnknownMembership,
    /// A membership operation used a different signed descriptor.
    DescriptorMismatch,
    /// A one-use prepared control no longer matches the document control generation.
    StaleControl,
    /// The configured hard membership ceiling was reached.
    MembershipLimit,
    /// The source baseline differed from the descriptor's signed baseline.
    BaselineMismatch,
    /// A source attempted to route an envelope to another logical subscription.
    RoutingMismatch,
    /// An envelope or wire frame violated the bounded canonical contract.
    InvalidEnvelope,
    /// A frame exceeded its fixed byte ceiling.
    FrameTooLarge,
    /// A frame used a binary, continuation, or fragmented shape that is unsupported.
    UnsupportedFrame,
    /// The host source failed without exposing provider detail.
    SourceFailed,
    /// The logical or document session is already closed.
    Closed,
}

impl AsyncTransportErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidOrigin => "invalid_origin",
            Self::OriginMismatch => "origin_mismatch",
            Self::TransportMismatch => "transport_mismatch",
            Self::AuthorizationScopeMismatch => "authorization_scope_mismatch",
            Self::AuthorizationLost => "authorization_lost",
            Self::DuplicateMembership => "duplicate_membership",
            Self::UnknownMembership => "unknown_membership",
            Self::DescriptorMismatch => "descriptor_mismatch",
            Self::StaleControl => "stale_control",
            Self::MembershipLimit => "membership_limit",
            Self::BaselineMismatch => "baseline_mismatch",
            Self::RoutingMismatch => "routing_mismatch",
            Self::InvalidEnvelope => "invalid_envelope",
            Self::FrameTooLarge => "frame_too_large",
            Self::UnsupportedFrame => "unsupported_frame",
            Self::SourceFailed => "source_failed",
            Self::Closed => "closed",
        }
    }
}

/// Safe typed transport failure that never contains descriptors or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncTransportError {
    kind: AsyncTransportErrorKind,
}

impl AsyncTransportError {
    /// Creates one closed transport failure.
    #[must_use]
    pub const fn new(kind: AsyncTransportErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> AsyncTransportErrorKind {
        self.kind
    }
}

impl fmt::Display for AsyncTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for AsyncTransportError {}

/// Closed failure returned by document-owned bounded delivery dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncDeliveryErrorKind {
    /// The document delivery owner was canceled or retired.
    Retired,
    /// Exact current logical membership or host authority was lost.
    AuthorizationLost,
    /// Task 3 rejected dispatch with its closed sequence failure kind.
    Sequence(SequenceErrorKind),
}

/// Redacted error from one document-owned delivery attempt.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AsyncDeliveryError {
    kind: AsyncDeliveryErrorKind,
    replay: Option<ReplayDispatchError>,
}

impl AsyncDeliveryError {
    const fn new(kind: AsyncDeliveryErrorKind) -> Self {
        Self { kind, replay: None }
    }

    const fn from_replay(error: ReplayDispatchError) -> Self {
        Self {
            kind: AsyncDeliveryErrorKind::Sequence(error.kind()),
            replay: Some(error),
        }
    }

    const fn with_replay(kind: AsyncDeliveryErrorKind, replay: ReplayDispatchError) -> Self {
        Self {
            kind,
            replay: Some(replay),
        }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> AsyncDeliveryErrorKind {
        self.kind
    }

    /// Returns truthful replay progress for any post-prepare failure kind.
    #[must_use]
    pub const fn replay_error(self) -> Option<ReplayDispatchError> {
        self.replay
    }
}

impl fmt::Display for AsyncDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            AsyncDeliveryErrorKind::Retired => formatter.write_str("async_delivery_retired"),
            AsyncDeliveryErrorKind::AuthorizationLost => {
                formatter.write_str("async_delivery_authorization_lost")
            }
            AsyncDeliveryErrorKind::Sequence(kind) => formatter.write_str(kind.as_str()),
        }
    }
}

impl fmt::Debug for AsyncDeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for AsyncDeliveryError {}

/// Truthful result of one closed document-owned delivery operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncDeliveryDisposition {
    /// One ordinary registered envelope passed through Task 3 sequence dispatch.
    Sequence(SequenceDisposition),
    /// One atomically admitted replay transcript completed Task 3 recovery.
    Replay(ReplayDispatchOutcome),
}

/// Canonical HTTP(S) origin proven free of path, query, fragment, or userinfo.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct VerifiedOrigin {
    scheme: &'static str,
    host: String,
    port: u16,
}

impl VerifiedOrigin {
    /// Parses and normalizes an exact origin serialization.
    pub fn parse(value: &str) -> Result<Self, AsyncTransportError> {
        if value.is_empty()
            || value.len() > 2_048
            || !value.is_ascii()
            || value.bytes().any(|byte| byte.is_ascii_control())
            || value.eq_ignore_ascii_case("null")
            || value == "*"
        {
            return Err(invalid_origin());
        }
        let authority_text = value
            .split_once("://")
            .map(|(_scheme, authority)| authority)
            .ok_or_else(invalid_origin)?;
        if authority_text
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'?' | b'#'))
        {
            return Err(invalid_origin());
        }
        let uri = http::Uri::from_str(value).map_err(|_| invalid_origin())?;
        let scheme = match uri.scheme_str() {
            Some("http") => "http",
            Some("https") => "https",
            _ => return Err(invalid_origin()),
        };
        let authority = uri.authority().ok_or_else(invalid_origin)?;
        if authority.as_str().contains('@') {
            return Err(invalid_origin());
        }
        let host = authority.host().to_ascii_lowercase();
        if !valid_origin_host(&host) {
            return Err(invalid_origin());
        }
        let port = authority
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        if port == 0 {
            return Err(invalid_origin());
        }
        Ok(Self { scheme, host, port })
    }

    /// Returns the normalized scheme.
    #[must_use]
    pub const fn scheme(&self) -> &'static str {
        self.scheme
    }

    /// Returns the normalized host, including brackets for an IPv6 literal.
    #[must_use]
    pub const fn host(&self) -> &str {
        self.host.as_str()
    }

    /// Returns the explicit or normalized default port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for VerifiedOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}://{}", self.scheme, self.host)?;
        let is_default = (self.scheme == "https" && self.port == 443)
            || (self.scheme == "http" && self.port == 80);
        if !is_default {
            write!(formatter, ":{}", self.port)?;
        }
        Ok(())
    }
}

impl fmt::Debug for VerifiedOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("VerifiedOrigin")
            .field(&self.to_string())
            .finish()
    }
}

/// Opaque non-authoritative correlation handle for one physical document transport.
#[derive(Clone, Eq, PartialEq)]
pub struct DocumentTransportHandle(Vec<u8>);

impl DocumentTransportHandle {
    /// Constructs a server-generated handle from fixed-width random bytes.
    pub fn from_bytes(value: &[u8]) -> Result<Self, AsyncTransportError> {
        if !(MIN_DOCUMENT_HANDLE_BYTES..=MAX_DOCUMENT_HANDLE_BYTES).contains(&value.len()) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        Ok(Self(value.to_vec()))
    }

    /// Parses one canonical unpadded base64url handle.
    pub fn parse(value: &str) -> Result<Self, AsyncTransportError> {
        if !(22..=43).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        let handle = Self::from_bytes(&bytes)?;
        if handle.to_base64url() != value {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        Ok(handle)
    }

    /// Returns the canonical non-secret correlation representation.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(&self.0)
    }
}

impl fmt::Debug for DocumentTransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<DocumentTransportHandle>")
    }
}

/// Hard bounded logical-membership policy for one document transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentTransportLimits {
    max_memberships: usize,
}

/// Physical document transport kind that logical memberships may not cross.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentTransportKind {
    /// One multiplexed Server-Sent Events response.
    ServerSentEvents,
    /// One multiplexed WebSocket connection.
    WebSocket,
}

impl DocumentTransportKind {
    const fn registration_mode(self) -> SubscriptionMode {
        match self {
            Self::ServerSentEvents => SubscriptionMode::ServerSentEvents,
            Self::WebSocket => SubscriptionMode::WebSocket,
        }
    }
}

/// External browser-control operation whose authority must be current at consumption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportMembershipOperation {
    /// Add one logical subscription to the exact document transport.
    Subscribe,
    /// Remove one logical subscription from the exact document transport.
    Unsubscribe,
}

/// Closed current facts a trusted host must re-evaluate for one control boundary.
#[derive(Clone, Copy)]
pub struct AsyncTransportAuthorityRequest<'a> {
    operation: TransportMembershipOperation,
    descriptor: &'a VerifiedSubscriptionDescriptor,
    binding: &'a SubscriptionBinding,
    subscription: &'a SubscriptionId,
    document_scope: &'a DocumentAuthorizationScope,
    document_origin: &'a VerifiedOrigin,
    document_kind: DocumentTransportKind,
    document_handle: &'a DocumentTransportHandle,
}

impl<'a> AsyncTransportAuthorityRequest<'a> {
    /// Returns the exact external membership operation being consumed.
    #[must_use]
    pub const fn operation(self) -> TransportMembershipOperation {
        self.operation
    }

    /// Returns the previously verified signed descriptor for current comparison.
    #[must_use]
    pub const fn descriptor(self) -> &'a VerifiedSubscriptionDescriptor {
        self.descriptor
    }

    /// Returns the compact binding of the exact signed descriptor wire.
    #[must_use]
    pub const fn binding(self) -> &'a SubscriptionBinding {
        self.binding
    }

    /// Returns the exact signed logical routing identity.
    #[must_use]
    pub const fn subscription(self) -> &'a SubscriptionId {
        self.subscription
    }

    /// Returns the trusted physical document authorization scope.
    #[must_use]
    pub const fn document_scope(self) -> &'a DocumentAuthorizationScope {
        self.document_scope
    }

    /// Returns the exact normalized origin owning the physical document transport.
    #[must_use]
    pub const fn document_origin(self) -> &'a VerifiedOrigin {
        self.document_origin
    }

    /// Returns the invoked adapter kind, which is compatibility rather than authority.
    #[must_use]
    pub const fn document_kind(self) -> DocumentTransportKind {
        self.document_kind
    }

    /// Returns the correlation-only document handle for exact host control lookup.
    #[must_use]
    pub const fn document_handle(self) -> &'a DocumentTransportHandle {
        self.document_handle
    }
}

impl fmt::Debug for AsyncTransportAuthorityRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsyncTransportAuthorityRequest")
            .field("operation", &self.operation)
            .field("subscription", &self.subscription)
            .field("document_origin", &self.document_origin)
            .field("document_kind", &self.document_kind)
            .field("document_handle", &self.document_handle)
            .field("document_scope", &self.document_scope)
            .field("binding", &self.binding)
            .field("descriptor", &"<redacted>")
            .finish()
    }
}

/// Framework-owned comparison sink for a trusted current-authority lookup.
///
/// Hosts may call `accept_current` only after re-resolving the current component
/// contract, registered stream/topics/events/modes, principal, session, tenant,
/// aggregate authorization scope, and exact document membership/control policy.
/// The authorization memo is Task 2's binding of component contract and host
/// scope; the remaining values are compared independently here.
pub struct AsyncTransportAuthorityValidation {
    expected_document_scope: DocumentAuthorizationScope,
    expected_memo: AuthorizationMemo,
    expected_stream: StreamName,
    expected_topics: BoundedTopics,
    expected_events: BoundedEventContracts,
    expected_modes: SubscriptionModes,
    required_mode: SubscriptionMode,
    outcome: Option<Result<(), AsyncTransportError>>,
}

impl AsyncTransportAuthorityValidation {
    /// Accepts a coherent fresh host snapshot; stale or drifted facts stay closed.
    pub fn accept_current(
        &mut self,
        document_scope: &DocumentAuthorizationScope,
        authorization_memo: &AuthorizationMemo,
        stream: &StreamName,
        topics: &BoundedTopics,
        events: &BoundedEventContracts,
        modes: &SubscriptionModes,
    ) -> bool {
        if self.outcome.is_some() {
            self.outcome = Some(Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            )));
            return false;
        }
        let exact_current = document_scope == &self.expected_document_scope
            && authorization_memo == &self.expected_memo
            && stream == &self.expected_stream
            && topics == &self.expected_topics
            && events == &self.expected_events
            && modes == &self.expected_modes;
        let outcome = if !exact_current {
            Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ))
        } else if !modes.as_slice().contains(&self.required_mode) {
            Err(AsyncTransportError::new(
                AsyncTransportErrorKind::TransportMismatch,
            ))
        } else {
            Ok(())
        };
        let accepted = outcome.is_ok();
        self.outcome = Some(outcome);
        accepted
    }

    fn finish(self) -> Result<(), AsyncTransportError> {
        self.outcome.unwrap_or_else(|| {
            Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ))
        })
    }
}

/// Trusted host port for fresh external membership authority and controlled time.
///
/// The port is called at every add/remove consumption boundary. A browser value,
/// document handle, or retained request can never construct a reusable accepted
/// guard. Internal completion, revocation cleanup, and shutdown do not use this
/// external-control port and retain cleanup authority after credentials expire.
pub trait AsyncTransportAuthorityPort: Send + Sync {
    /// Returns current host time for exclusive descriptor-expiry enforcement.
    fn now(&self) -> UnixMillis;

    /// Re-evaluates all current registry, identity, and exact document-control facts.
    fn validate_current<'a>(
        &'a self,
        request: AsyncTransportAuthorityRequest<'a>,
        validation: &'a mut AsyncTransportAuthorityValidation,
    ) -> AsyncTransportFuture<'a, ()>;
}

impl DocumentTransportLimits {
    /// Creates a non-zero limit no greater than the architecture ceiling.
    pub fn new(max_memberships: usize) -> Result<Self, AsyncTransportError> {
        if max_memberships == 0 || max_memberships > MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::MembershipLimit,
            ));
        }
        Ok(Self { max_memberships })
    }

    /// Returns the maximum logical memberships in this physical transport.
    #[must_use]
    pub const fn max_memberships(self) -> usize {
        self.max_memberships
    }
}

#[derive(Clone)]
struct DocumentControlSnapshot {
    owner: Arc<()>,
    generation: u64,
    origin: VerifiedOrigin,
    kind: DocumentTransportKind,
    handle: DocumentTransportHandle,
    authorization_scope: DocumentAuthorizationScope,
}

impl DocumentControlSnapshot {
    fn validate_physical(
        &self,
        document: &DocumentTransportSession,
    ) -> Result<(), AsyncTransportError> {
        if document.closed || document.closing || document.generation_exhausted {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        if !Arc::ptr_eq(&self.owner, &document.control_owner) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::StaleControl,
            ));
        }
        if self.origin != document.origin {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::OriginMismatch,
            ));
        }
        if self.kind != document.kind {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::TransportMismatch,
            ));
        }
        if self.handle != document.handle {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::RoutingMismatch,
            ));
        }
        if self.authorization_scope != document.authorization_scope {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationScopeMismatch,
            ));
        }
        if self.generation != document.control_generation {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::StaleControl,
            ));
        }
        Ok(())
    }
}

/// Descriptor-bound membership request that grants no reusable current authority.
#[derive(Clone)]
pub struct AuthorizedTransportSubscription {
    context: AsyncEnvelopeContext,
    verified: VerifiedSubscriptionDescriptor,
    binding: SubscriptionBinding,
    document_scope: DocumentAuthorizationScope,
    origin: VerifiedOrigin,
    authorized_modes: SubscriptionModes,
    authority: Arc<dyn AsyncTransportAuthorityPort>,
}

impl AuthorizedTransportSubscription {
    /// Binds Task 2 authorization to one logical membership and exact origin.
    ///
    /// `authorized_modes` must come from the same trusted registered metadata
    /// resolved for Task 2 authorization. Construction captures descriptor-bound
    /// facts only; it does not replace the fresh authority-port checks performed
    /// whenever an external add or remove consumes this request.
    #[allow(
        clippy::too_many_arguments,
        reason = "transport admission keeps each independently trusted authority input explicit"
    )]
    pub fn new(
        authorized: &AuthorizedSubscription,
        subscription: SubscriptionId,
        registry: &dyn AsyncMembershipRegistryPort,
        origin: VerifiedOrigin,
        document_scope: DocumentAuthorizationScope,
        authorized_modes: SubscriptionModes,
        authority: Arc<dyn AsyncTransportAuthorityPort>,
        now: UnixMillis,
    ) -> Result<Self, AsyncTransportError> {
        if now >= authorized.verified().expires_at() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        let context = AsyncEnvelopeContext::from_authorized(authorized, subscription, registry)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost))?;
        Ok(Self {
            context,
            verified: authorized.verified().clone(),
            binding: authorized.binding().clone(),
            document_scope,
            origin,
            authorized_modes,
            authority,
        })
    }

    /// Returns the exact active logical subscription identity.
    #[must_use]
    pub const fn subscription(&self) -> &SubscriptionId {
        self.context.subscription()
    }

    /// Returns the exact descriptor baseline for this membership.
    #[must_use]
    pub const fn baseline(&self) -> StreamPosition {
        self.context.authoritative_baseline()
    }

    /// Returns the sealed envelope and sequence context.
    #[must_use]
    pub const fn context(&self) -> &AsyncEnvelopeContext {
        &self.context
    }

    /// Returns the exact normalized physical transport origin.
    #[must_use]
    pub const fn origin(&self) -> &VerifiedOrigin {
        &self.origin
    }

    /// Returns the compact binding of the exact signed descriptor wire.
    #[must_use]
    pub const fn binding(&self) -> &SubscriptionBinding {
        &self.binding
    }

    /// Returns the trusted physical document sharing scope.
    #[must_use]
    pub const fn document_scope(&self) -> &DocumentAuthorizationScope {
        &self.document_scope
    }

    async fn validate_current(
        &self,
        document: &DocumentControlSnapshot,
        operation: TransportMembershipOperation,
    ) -> Result<(), AsyncTransportError> {
        if self.authority.now() >= self.verified.expires_at() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        let claims = self.verified.claims();
        let mut validation = AsyncTransportAuthorityValidation {
            expected_document_scope: self.document_scope.clone(),
            expected_memo: claims.authorization_memo().clone(),
            expected_stream: claims.stream().clone(),
            expected_topics: claims.topics().clone(),
            expected_events: claims.events().clone(),
            expected_modes: self.authorized_modes.clone(),
            required_mode: document.kind.registration_mode(),
            outcome: None,
        };
        self.authority
            .validate_current(
                AsyncTransportAuthorityRequest {
                    operation,
                    descriptor: &self.verified,
                    binding: &self.binding,
                    subscription: self.subscription(),
                    document_scope: &document.authorization_scope,
                    document_origin: &document.origin,
                    document_kind: document.kind,
                    document_handle: &document.handle,
                },
                &mut validation,
            )
            .await;
        if self.authority.now() >= self.verified.expires_at() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        validation.finish()
    }
}

impl fmt::Debug for AuthorizedTransportSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizedTransportSubscription:redacted>")
    }
}

/// Host-neutral source for one currently authorized logical subscription.
pub trait AsyncEventSource: Send + Sync {
    /// Establishes one logical session from an exact Task 2 authorization result.
    ///
    /// Subscription establishment must be cancellation-safe: dropping this
    /// future cannot leave an unowned provider subscription behind.
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>>;
}

/// Host-neutral session for one currently authorized logical subscription.
///
/// Persistent polling avoids allocating and dropping one boxed future per
/// membership on every document wake. Implementations must not consume or
/// reorder a message when a poll returns `Pending`, and must release provider
/// resources when dropped; `poll_close` is the explicit graceful-shutdown path.
pub trait AsyncEventSession: Send {
    /// Returns the exact authoritative baseline bound to this logical session.
    fn baseline(&self) -> StreamPosition;

    /// Returns the next bounded authorized envelope, or `None` after completion.
    fn poll_next(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>>;

    /// Closes this logical session idempotently.
    ///
    /// A `Pending` or failed close retains cleanup authority in the document's
    /// bounded retirement lane and may be polled again without duplicating the
    /// logical close transition.
    fn poll_close(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>>;
}

struct UncommittedSession {
    session: Option<Pin<Box<dyn AsyncEventSession>>>,
}

#[derive(Default)]
struct PendingControlCapacity {
    in_flight: AtomicUsize,
}

impl PendingControlCapacity {
    fn acquire(self: &Arc<Self>) -> Result<PendingControlPermit, AsyncTransportError> {
        let mut observed = self.in_flight.load(Ordering::Acquire);
        loop {
            if observed >= MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS {
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::MembershipLimit,
                ));
            }
            match self.in_flight.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(PendingControlPermit {
                        capacity: Arc::clone(self),
                    });
                }
                Err(actual) => observed = actual,
            }
        }
    }
}

struct PendingControlPermit {
    capacity: Arc<PendingControlCapacity>,
}

impl Drop for PendingControlPermit {
    fn drop(&mut self) {
        let previous = self.capacity.in_flight.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "pending control permit count underflowed");
    }
}

impl UncommittedSession {
    fn new(session: Pin<Box<dyn AsyncEventSession>>) -> Self {
        Self {
            session: Some(session),
        }
    }

    fn take(&mut self) -> Option<Pin<Box<dyn AsyncEventSession>>> {
        self.session.take()
    }
}

impl Drop for UncommittedSession {
    fn drop(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        let waker = std::task::Waker::noop();
        let mut task = Context::from_waker(waker);
        let _ = session.as_mut().poll_close(&mut task);
    }
}

/// One-use add operation whose fresh authority phase borrows no document session.
pub struct PendingTransportAdd {
    document: DocumentControlSnapshot,
    authorization: AuthorizedTransportSubscription,
    permit: PendingControlPermit,
}

impl PendingTransportAdd {
    /// Performs fresh pre-source authority without classifying document membership state.
    pub async fn authorize(self) -> Result<AuthorizedTransportAdd, AsyncTransportError> {
        self.authorization
            .validate_current(&self.document, TransportMembershipOperation::Subscribe)
            .await?;
        Ok(AuthorizedTransportAdd {
            document: self.document,
            authorization: self.authorization,
            permit: self.permit,
        })
    }
}

impl fmt::Debug for PendingTransportAdd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PendingTransportAdd:redacted>")
    }
}

/// Freshly authorized add awaiting a synchronous pre-source document gate.
pub struct AuthorizedTransportAdd {
    document: DocumentControlSnapshot,
    authorization: AuthorizedTransportSubscription,
    permit: PendingControlPermit,
}

impl fmt::Debug for AuthorizedTransportAdd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizedTransportAdd:redacted>")
    }
}

/// One-use logical-source establishment that owns no document borrow.
pub struct EstablishingTransportAdd {
    document: DocumentControlSnapshot,
    authorization: AuthorizedTransportSubscription,
    permit: PendingControlPermit,
}

impl EstablishingTransportAdd {
    /// Opens a logical source and rechecks current authority after the await.
    pub async fn establish(
        self,
        source: &dyn AsyncEventSource,
    ) -> Result<ReadyTransportAdd, AsyncTransportError> {
        let session = source.subscribe(&self.authorization).await?;
        let mut cleanup = UncommittedSession::new(session);
        let baseline_matches = cleanup
            .session
            .as_ref()
            .is_some_and(|session| session.baseline() == self.authorization.baseline());
        if !baseline_matches {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::BaselineMismatch,
            ));
        }
        self.authorization
            .validate_current(&self.document, TransportMembershipOperation::Subscribe)
            .await?;
        Ok(ReadyTransportAdd {
            document: self.document,
            authorization: self.authorization,
            permit: self.permit,
            session: UncommittedSession::new(
                cleanup.take().ok_or_else(|| {
                    AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed)
                })?,
            ),
        })
    }
}

impl fmt::Debug for EstablishingTransportAdd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<EstablishingTransportAdd:redacted>")
    }
}

/// One-use capability ready for a synchronous exact document commit.
pub struct ReadyTransportAdd {
    document: DocumentControlSnapshot,
    authorization: AuthorizedTransportSubscription,
    permit: PendingControlPermit,
    session: UncommittedSession,
}

impl fmt::Debug for ReadyTransportAdd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ReadyTransportAdd:redacted>")
    }
}

/// One-use removal operation whose asynchronous authority phase borrows no document.
pub struct PendingTransportRemove<'a> {
    document: DocumentControlSnapshot,
    authorization: &'a AuthorizedTransportSubscription,
    permit: PendingControlPermit,
}

impl PendingTransportRemove<'_> {
    /// Rechecks current authenticated removal authority without document state classification.
    pub async fn authorize(self) -> Result<ReadyTransportRemove, AsyncTransportError> {
        self.authorization
            .validate_current(&self.document, TransportMembershipOperation::Unsubscribe)
            .await?;
        Ok(ReadyTransportRemove {
            document: self.document,
            subscription: self.authorization.subscription().clone(),
            binding: self.authorization.binding.clone(),
            expires_at: self.authorization.verified.expires_at(),
            authority: self.authorization.authority.clone(),
            permit: self.permit,
        })
    }
}

impl fmt::Debug for PendingTransportRemove<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PendingTransportRemove:redacted>")
    }
}

/// One-use capability ready for synchronous authenticated removal classification.
pub struct ReadyTransportRemove {
    document: DocumentControlSnapshot,
    subscription: SubscriptionId,
    binding: SubscriptionBinding,
    expires_at: UnixMillis,
    authority: Arc<dyn AsyncTransportAuthorityPort>,
    permit: PendingControlPermit,
}

impl fmt::Debug for ReadyTransportRemove {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ReadyTransportRemove:redacted>")
    }
}

struct LogicalTransportSession {
    authorization: AuthorizedTransportSubscription,
    session: Pin<Box<dyn AsyncEventSession>>,
}

struct RetiringTransportSession {
    authorization: SubscriptionBinding,
    subscription: SubscriptionId,
    session: Pin<Box<dyn AsyncEventSession>>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "the sealed candidate is a single transient poll result; boxing would add a heap allocation to every delivery"
)]
enum DocumentPoll {
    Envelope(DocumentDeliveryCandidate),
    Retired,
    Empty,
}

struct DocumentDeliveryCandidate {
    authorization: AuthorizedTransportSubscription,
    envelope: AsyncEnvelope,
    terminal: bool,
}

/// Bounded fan-in owner for compatible logical subscriptions.
pub struct DocumentTransportSession {
    origin: VerifiedOrigin,
    kind: DocumentTransportKind,
    handle: DocumentTransportHandle,
    limits: DocumentTransportLimits,
    authorization_scope: DocumentAuthorizationScope,
    control_owner: Arc<()>,
    control_generation: u64,
    generation_exhausted: bool,
    pending_controls: Arc<PendingControlCapacity>,
    memberships: Vec<LogicalTransportSession>,
    retiring: Vec<RetiringTransportSession>,
    completed_drains: Vec<AuthorizedTransportSubscription>,
    cursor: usize,
    cleanup_cursor: usize,
    last_cleanup_error: Option<AsyncTransportErrorKind>,
    closing: bool,
    closed: bool,
}

impl DocumentTransportSession {
    /// Creates one physical transport owner with no active logical memberships.
    #[must_use]
    pub fn new(
        origin: VerifiedOrigin,
        kind: DocumentTransportKind,
        handle: DocumentTransportHandle,
        limits: DocumentTransportLimits,
        authorization_scope: DocumentAuthorizationScope,
    ) -> Self {
        Self {
            origin,
            kind,
            handle,
            limits,
            authorization_scope,
            control_owner: Arc::new(()),
            control_generation: 0,
            generation_exhausted: false,
            pending_controls: Arc::new(PendingControlCapacity::default()),
            memberships: Vec::new(),
            retiring: Vec::new(),
            completed_drains: Vec::new(),
            cursor: 0,
            cleanup_cursor: 0,
            last_cleanup_error: None,
            closing: false,
            closed: false,
        }
    }

    /// Returns the exact origin shared by every compatible membership.
    #[must_use]
    pub const fn origin(&self) -> &VerifiedOrigin {
        &self.origin
    }

    /// Returns the exact physical transport kind shared by every membership.
    #[must_use]
    pub const fn kind(&self) -> DocumentTransportKind {
        self.kind
    }

    /// Returns the non-authoritative correlation handle.
    #[must_use]
    pub const fn handle(&self) -> &DocumentTransportHandle {
        &self.handle
    }

    /// Returns the number of independently authorized logical sessions.
    #[must_use]
    pub fn membership_count(&self) -> usize {
        self.memberships.len()
    }

    /// Returns the bounded logical sessions detached from routing but still cleaning up.
    #[must_use]
    pub fn retiring_count(&self) -> usize {
        self.retiring.len()
    }

    /// Returns the most recently observed provider cleanup failure, if any.
    #[must_use]
    pub const fn last_cleanup_error(&self) -> Option<AsyncTransportErrorKind> {
        self.last_cleanup_error
    }

    /// Returns whether one logical subscription is currently multiplexed here.
    #[must_use]
    pub fn contains_membership(&self, subscription: &SubscriptionId) -> bool {
        self.memberships
            .iter()
            .any(|logical| logical.authorization.subscription() == subscription)
    }

    fn seal_async_delivery(
        &self,
        authorization: &AuthorizedTransportSubscription,
        envelope: AsyncEnvelope,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
        terminal: bool,
        require_active: bool,
    ) -> Result<AuthorizedAsyncBufferEntry, AsyncTransportError> {
        self.validate_common(authorization)?;
        let active = self
            .memberships
            .iter()
            .find(|logical| logical.authorization.subscription() == authorization.subscription());
        if require_active {
            let logical = active.ok_or_else(|| {
                AsyncTransportError::new(AsyncTransportErrorKind::UnknownMembership)
            })?;
            if logical.authorization.binding() != authorization.binding() {
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::DescriptorMismatch,
                ));
            }
        }
        let membership = authorization
            .context
            .admit_owned(
                envelope,
                &authorization.binding,
                &authorization.document_scope,
                registry,
                now,
            )
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost))?;
        let resolved_fanout = membership
            .resolved_event()
            .map_or(1, |resolved| usize::from(resolved.recipients().get()));
        Ok(AuthorizedAsyncBufferEntry::new(
            membership,
            authorization.binding.clone(),
            authorization.document_scope.clone(),
            authorization.verified.claims().authorization_memo().clone(),
            self.control_generation,
            resolved_fanout,
            terminal,
        ))
    }

    fn seal_async_replay(
        &self,
        authorization: &AuthorizedTransportSubscription,
        envelopes: Vec<AsyncEnvelope>,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
    ) -> Result<Vec<AuthorizedAsyncBufferEntry>, AsyncTransportError> {
        self.validate_common(authorization)?;
        let logical = self
            .memberships
            .iter()
            .find(|logical| logical.authorization.subscription() == authorization.subscription())
            .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::UnknownMembership))?;
        if logical.authorization.binding() != authorization.binding() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::DescriptorMismatch,
            ));
        }
        let memberships = authorization
            .context
            .admit_replay_owned(
                envelopes,
                authorization.binding(),
                authorization.document_scope(),
                registry,
                now,
            )
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost))?;
        Ok(memberships
            .into_iter()
            .map(|membership| {
                let resolved_fanout = membership
                    .resolved_event()
                    .map_or(1, |resolved| usize::from(resolved.recipients().get()));
                AuthorizedAsyncBufferEntry::new(
                    membership,
                    authorization.binding.clone(),
                    authorization.document_scope.clone(),
                    authorization.verified.claims().authorization_memo().clone(),
                    self.control_generation,
                    resolved_fanout,
                    false,
                )
            })
            .collect())
    }

    fn revalidate_async_delivery(
        &self,
        authorization: &AuthorizedTransportSubscription,
        authorized: &mut AuthorizedAsyncBufferEntry,
        registry: &dyn AsyncMembershipRegistryPort,
        now: UnixMillis,
        require_active: bool,
    ) -> Result<(), AsyncTransportError> {
        self.validate_common(authorization)?;
        if require_active {
            let logical = self
                .memberships
                .iter()
                .find(|logical| {
                    logical.authorization.subscription() == authorization.subscription()
                })
                .ok_or_else(|| {
                    AsyncTransportError::new(AsyncTransportErrorKind::UnknownMembership)
                })?;
            if logical.authorization.binding() != authorization.binding() {
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::DescriptorMismatch,
                ));
            }
        }
        let component_memo = authorization.verified.claims().authorization_memo();
        if !authorized.matches_authorization(
            authorization.binding(),
            authorization.document_scope(),
            component_memo,
            authorization.context(),
        ) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::DescriptorMismatch,
            ));
        }
        let fresh = authorization
            .context
            .admit_owned(
                authorized.envelope().clone(),
                authorization.binding(),
                authorization.document_scope(),
                registry,
                now,
            )
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost))?;
        if !authorized.current_resolution_matches(&fresh) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        authorized.replace_membership(fresh);
        authorized.replace_document_generation(self.control_generation);
        Ok(())
    }

    /// Snapshots one descriptor-bound admission without borrowing this document across await.
    pub fn prepare_add(
        &self,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<PendingTransportAdd, AsyncTransportError> {
        self.validate_common(&authorization)?;
        let permit = self.pending_controls.acquire()?;
        Ok(PendingTransportAdd {
            document: self.control_snapshot(),
            authorization,
            permit,
        })
    }

    /// Rechecks document generation, duplicate fences, and capacity before source work.
    pub fn prepare_establish(
        &self,
        authorized: AuthorizedTransportAdd,
    ) -> Result<EstablishingTransportAdd, AsyncTransportError> {
        self.validate_ready_control(
            &authorized.document,
            authorized.authorization.verified.expires_at(),
            authorized.authorization.authority.as_ref(),
        )?;
        self.validate_ready_add(
            authorized.authorization.subscription(),
            authorized.authorization.binding(),
        )?;
        Ok(EstablishingTransportAdd {
            document: self.control_snapshot(),
            authorization: authorized.authorization,
            permit: authorized.permit,
        })
    }

    /// Commits one freshly established logical session synchronously and exactly once.
    pub fn commit_add(&mut self, mut ready: ReadyTransportAdd) -> Result<(), AsyncTransportError> {
        self.validate_ready_control(
            &ready.document,
            ready.authorization.verified.expires_at(),
            ready.authorization.authority.as_ref(),
        )?;
        self.validate_ready_add(
            ready.authorization.subscription(),
            ready.authorization.binding(),
        )?;
        self.ensure_generation_available()?;
        let _permit = ready.permit;
        let session = ready
            .session
            .take()
            .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed))?;
        self.memberships.push(LogicalTransportSession {
            authorization: ready.authorization,
            session,
        });
        self.advance_control_generation();
        Ok(())
    }

    /// Snapshots authenticated removal without classifying local membership state.
    pub fn prepare_remove<'a>(
        &self,
        authorization: &'a AuthorizedTransportSubscription,
    ) -> Result<PendingTransportRemove<'a>, AsyncTransportError> {
        self.validate_common(authorization)?;
        let permit = self.pending_controls.acquire()?;
        Ok(PendingTransportRemove {
            document: self.control_snapshot(),
            authorization,
            permit,
        })
    }

    /// Classifies and commits one freshly authorized removal synchronously.
    ///
    /// Provider cleanup is polled once immediately and otherwise remains owned
    /// by the bounded retirement lane, so it cannot block healthy siblings.
    pub fn commit_remove(
        &mut self,
        ready: ReadyTransportRemove,
    ) -> Result<CloseDisposition, AsyncTransportError> {
        self.validate_ready_control(&ready.document, ready.expires_at, ready.authority.as_ref())?;
        let _permit = ready.permit;
        let Some(index) = self
            .memberships
            .iter()
            .position(|logical| logical.authorization.subscription() == &ready.subscription)
        else {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::UnknownMembership,
            ));
        };
        if self.memberships[index].authorization.binding() != &ready.binding {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::DescriptorMismatch,
            ));
        }
        self.ensure_generation_available()?;
        let retirement = self.detach_membership(index);
        self.poll_exact_retirement_once(retirement);
        Ok(CloseDisposition::Closed)
    }

    async fn next_delivery_candidate(
        &mut self,
    ) -> Result<Option<DocumentDeliveryCandidate>, AsyncTransportError> {
        if self.closed || self.closing {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        loop {
            match poll_fn(|task| self.poll_document_next(task)).await? {
                DocumentPoll::Envelope(candidate) => return Ok(Some(candidate)),
                DocumentPoll::Retired => {}
                DocumentPoll::Empty => return Ok(None),
            }
        }
    }

    fn begin_bounded_close(&mut self) {
        if self.closed || self.closing {
            return;
        }
        self.closing = true;
        while !self.memberships.is_empty() {
            self.detach_membership(self.memberships.len() - 1);
        }
        self.poll_bounded_close_once();
    }

    fn poll_bounded_close_once(&mut self) {
        if !self.closing {
            return;
        }
        let waker = std::task::Waker::noop();
        let mut task = Context::from_waker(waker);
        let _ = self.poll_all_retirements_once(&mut task);
        if self.retiring.is_empty() {
            self.closed = true;
            self.closing = false;
        }
    }

    /// Closes every logical session and the document owner exactly once.
    pub async fn close(&mut self) -> Result<CloseDisposition, AsyncTransportError> {
        if self.closed {
            return Ok(CloseDisposition::AlreadyClosed);
        }
        self.closing = true;
        while !self.memberships.is_empty() {
            self.detach_membership(self.memberships.len() - 1);
        }
        poll_fn(|task| {
            let error = self.poll_all_retirements_once(task);
            if self.retiring.is_empty() {
                self.closed = true;
                self.closing = false;
                Poll::Ready(Ok(CloseDisposition::Closed))
            } else if let Some(error) = error {
                Poll::Ready(Err(error))
            } else {
                Poll::Pending
            }
        })
        .await
    }

    fn validate_ready_add(
        &self,
        subscription: &SubscriptionId,
        binding: &SubscriptionBinding,
    ) -> Result<(), AsyncTransportError> {
        let active = self.memberships.iter().map(|logical| {
            (
                logical.authorization.subscription(),
                logical.authorization.binding(),
            )
        });
        let retiring = self
            .retiring
            .iter()
            .map(|logical| (&logical.subscription, &logical.authorization));
        if let Some((_, existing_binding)) = active
            .chain(retiring)
            .find(|(existing, _)| *existing == subscription)
        {
            let kind = if existing_binding == binding {
                AsyncTransportErrorKind::DuplicateMembership
            } else {
                AsyncTransportErrorKind::DescriptorMismatch
            };
            return Err(AsyncTransportError::new(kind));
        }
        if self.memberships.len() + self.retiring.len() >= self.limits.max_memberships() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::MembershipLimit,
            ));
        }
        Ok(())
    }

    fn validate_common(
        &self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<(), AsyncTransportError> {
        if self.closed || self.closing || self.generation_exhausted {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        if authorization.origin() != &self.origin {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::OriginMismatch,
            ));
        }
        if self.authorization_scope != *authorization.document_scope() {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationScopeMismatch,
            ));
        }
        Ok(())
    }

    fn resolve_active_stored_authorization(
        &self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<AuthorizedTransportSubscription, AsyncTransportError> {
        self.validate_common(authorization)?;
        let stored = self
            .memberships
            .iter()
            .find(|logical| {
                logical.authorization.subscription() == authorization.subscription()
                    && logical.authorization.binding() == authorization.binding()
                    && logical.authorization.document_scope() == authorization.document_scope()
            })
            .map(|logical| logical.authorization.clone())
            .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        if stored.context() != authorization.context()
            || stored.origin() != authorization.origin()
            || stored.document_scope() != authorization.document_scope()
            || stored.binding() != authorization.binding()
            || !Arc::ptr_eq(&stored.authority, &authorization.authority)
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        Ok(stored)
    }

    fn control_snapshot(&self) -> DocumentControlSnapshot {
        DocumentControlSnapshot {
            owner: Arc::clone(&self.control_owner),
            generation: self.control_generation,
            origin: self.origin.clone(),
            kind: self.kind,
            handle: self.handle.clone(),
            authorization_scope: self.authorization_scope.clone(),
        }
    }

    fn validate_ready_control(
        &self,
        snapshot: &DocumentControlSnapshot,
        expires_at: UnixMillis,
        authority: &dyn AsyncTransportAuthorityPort,
    ) -> Result<(), AsyncTransportError> {
        snapshot.validate_physical(self)?;
        if authority.now() >= expires_at {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        Ok(())
    }

    fn ensure_generation_available(&self) -> Result<(), AsyncTransportError> {
        if self.control_generation == u64::MAX {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        Ok(())
    }

    fn advance_control_generation(&mut self) {
        match self.control_generation.checked_add(1) {
            Some(generation) => self.control_generation = generation,
            None => self.generation_exhausted = true,
        }
    }

    fn repair_cursor(&mut self) {
        if self.memberships.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor %= self.memberships.len();
        }
    }

    fn repair_cursor_after_removal(&mut self, removed: usize) {
        if removed < self.cursor {
            self.cursor -= 1;
        }
        self.repair_cursor();
    }

    fn detach_membership(&mut self, index: usize) -> usize {
        let logical = self.memberships.remove(index);
        self.repair_cursor_after_removal(index);
        self.retiring.push(RetiringTransportSession {
            authorization: logical.authorization.binding.clone(),
            subscription: logical.authorization.subscription().clone(),
            session: logical.session,
        });
        self.advance_control_generation();
        self.retiring.len() - 1
    }

    fn poll_exact_retirement_once(&mut self, index: usize) {
        let waker = std::task::Waker::noop();
        let mut task = Context::from_waker(waker);
        self.poll_retirement_at(index, &mut task);
    }

    fn poll_retirement_at(&mut self, index: usize, task: &mut Context<'_>) {
        if index >= self.retiring.len() {
            return;
        }
        match self.retiring[index].session.as_mut().poll_close(task) {
            Poll::Ready(Ok(_)) => {
                self.retiring.remove(index);
                self.repair_cleanup_cursor();
                self.advance_control_generation();
            }
            Poll::Ready(Err(error)) => {
                self.last_cleanup_error = Some(error.kind());
                self.cleanup_cursor = (index + 1) % self.retiring.len();
            }
            Poll::Pending => {
                self.cleanup_cursor = (index + 1) % self.retiring.len();
            }
        }
    }

    fn poll_one_retirement(&mut self, task: &mut Context<'_>) {
        if self.retiring.is_empty() {
            return;
        }
        let index = self.cleanup_cursor % self.retiring.len();
        self.poll_retirement_at(index, task);
    }

    fn poll_all_retirements_once(&mut self, task: &mut Context<'_>) -> Option<AsyncTransportError> {
        let target = self.retiring.len();
        let mut first_error = None;
        for _ in 0..target {
            if self.retiring.is_empty() {
                break;
            }
            let index = self.cleanup_cursor % self.retiring.len();
            match self.retiring[index].session.as_mut().poll_close(task) {
                Poll::Ready(Ok(_)) => {
                    self.retiring.remove(index);
                    self.repair_cleanup_cursor();
                    self.advance_control_generation();
                }
                Poll::Ready(Err(error)) => {
                    self.last_cleanup_error = Some(error.kind());
                    first_error.get_or_insert(error);
                    self.cleanup_cursor = (index + 1) % self.retiring.len();
                }
                Poll::Pending => {
                    self.cleanup_cursor = (index + 1) % self.retiring.len();
                }
            }
        }
        first_error
    }

    fn poll_document_next(
        &mut self,
        task: &mut Context<'_>,
    ) -> Poll<Result<DocumentPoll, AsyncTransportError>> {
        self.poll_one_retirement(task);
        if self.memberships.is_empty() {
            return Poll::Ready(Ok(DocumentPoll::Empty));
        }
        let count = self.memberships.len();
        for offset in 0..count {
            let index = (self.cursor + offset) % self.memberships.len();
            let result = self.memberships[index].session.as_mut().poll_next(task);
            let Poll::Ready(result) = result else {
                continue;
            };
            match result {
                Err(error) => {
                    let retirement = self.detach_membership(index);
                    self.poll_retirement_at(retirement, task);
                    return Poll::Ready(Err(error));
                }
                Ok(None) => {
                    let authorization = self.memberships[index].authorization.clone();
                    self.completed_drains.push(authorization);
                    let retirement = self.detach_membership(index);
                    self.poll_retirement_at(retirement, task);
                    return Poll::Ready(Ok(DocumentPoll::Retired));
                }
                Ok(Some(envelope)) => {
                    if envelope.subscription()
                        != self.memberships[index].authorization.subscription()
                    {
                        let retirement = self.detach_membership(index);
                        self.poll_retirement_at(retirement, task);
                        return Poll::Ready(Err(AsyncTransportError::new(
                            AsyncTransportErrorKind::RoutingMismatch,
                        )));
                    }
                    let terminal = matches!(envelope.payload(), AsyncPayload::Complete(_));
                    let authorization = self.memberships[index].authorization.clone();
                    if terminal {
                        let retirement = self.detach_membership(index);
                        self.poll_retirement_at(retirement, task);
                    } else {
                        self.cursor = (index + 1) % self.memberships.len();
                    }
                    return Poll::Ready(Ok(DocumentPoll::Envelope(DocumentDeliveryCandidate {
                        authorization,
                        envelope,
                        terminal,
                    })));
                }
            }
        }
        Poll::Pending
    }

    fn repair_cleanup_cursor(&mut self) {
        if self.retiring.is_empty() {
            self.cleanup_cursor = 0;
        } else {
            self.cleanup_cursor %= self.retiring.len();
        }
    }
}

impl fmt::Debug for DocumentTransportSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentTransportSession")
            .field("origin", &self.origin)
            .field("kind", &self.kind)
            .field("handle", &self.handle)
            .field("membership_count", &self.memberships.len())
            .field("retiring_count", &self.retiring.len())
            .field("max_memberships", &self.limits.max_memberships())
            .field("closing", &self.closing)
            .field("closed", &self.closed)
            .finish()
    }
}

/// One physical document transport composed with one aggregate delivery queue.
///
/// `pump_next` pulls at most one provider item and immediately admits it to the
/// shared bounded queue. The wrapper owns no ingress staging buffer; buffering
/// performed inside a host-native source remains outside the transport trait's
/// framework-owned memory contract.
///
/// Raw admission capabilities are intentionally not part of the public API:
///
/// ```compile_fail
/// use suprnova_live::async_updates::{AsyncBackpressure, AuthorizedAsyncBufferEntry};
/// ```
///
/// ```compile_fail
/// use suprnova_live::async_updates::DocumentTransportSession;
///
/// async fn bypass(session: &mut DocumentTransportSession) {
///     let _ = session.next().await;
/// }
/// ```
///
/// Static decode context cannot mint a production delivery guard:
///
/// ```compile_fail
/// use suprnova_live::async_updates::{
///     AsyncEnvelope, AsyncEnvelopeContext, AsyncMembershipRegistryPort,
/// };
/// use suprnova_live::identity::UnixMillis;
///
/// fn bypass(
///     context: &AsyncEnvelopeContext,
///     envelope: &AsyncEnvelope,
///     registry: &dyn AsyncMembershipRegistryPort,
/// ) {
///     let _ = context.admit(envelope, registry, UnixMillis::new(0));
/// }
/// ```
///
/// Raw sequence authority is likewise not an application-facing constructor:
///
/// ```compile_fail
/// use suprnova_live::async_updates::{AsyncEnvelopeContext, SequenceMachine};
///
/// fn bypass(context: &AsyncEnvelopeContext) {
///     let _ = SequenceMachine::new(context);
/// }
/// ```
///
/// A host dispatcher receives, but cannot forge, resolved delivery authority:
///
/// ```compile_fail
/// use suprnova_live::async_updates::ResolvedAsyncDelivery;
///
/// fn forge() {
///     let _ = ResolvedAsyncDelivery {
///         guard: todo!(),
///         resolved_event: None,
///         deployment_fanout_limit: 100,
///     };
/// }
/// ```
///
/// The one-use resolved proof cannot be duplicated:
///
/// ```compile_fail
/// use suprnova_live::async_updates::ResolvedAsyncDelivery;
///
/// fn duplicate(delivery: ResolvedAsyncDelivery<'_>) {
///     let _ = delivery.clone();
/// }
/// ```
pub struct BoundedDocumentTransportSession {
    transport: DocumentTransportSession,
    pressure: AsyncBackpressure,
    terminal_drains: Vec<AuthorizedTransportSubscription>,
    sequence_lanes: Vec<DocumentSequenceLane>,
}

struct DocumentSequenceLane {
    subscription: SubscriptionId,
    binding: SubscriptionBinding,
    machine: SequenceMachine,
}

impl DocumentSequenceLane {
    fn new(authorization: &AuthorizedTransportSubscription) -> Self {
        Self {
            subscription: authorization.subscription().clone(),
            binding: authorization.binding().clone(),
            machine: SequenceMachine::new(authorization.context()),
        }
    }

    fn matches(&self, authorization: &AuthorizedTransportSubscription) -> bool {
        self.subscription == *authorization.subscription()
            && self.binding == *authorization.binding()
    }
}

impl BoundedDocumentTransportSession {
    /// Composes Task 4 fair fan-in with one Task 5 queue and shared permit pool.
    pub fn new(
        transport: DocumentTransportSession,
        bounds: ResourceBounds,
        permits: PermitPool,
        policy: AsyncPolicy,
    ) -> Result<Self, AsyncBackpressureError> {
        let sequence_lanes = transport
            .memberships
            .iter()
            .map(|logical| DocumentSequenceLane::new(&logical.authorization))
            .collect();
        Ok(Self {
            transport,
            pressure: AsyncBackpressure::new(
                ResourceOwner::<AsyncBufferEntry>::new(bounds),
                permits,
                policy,
            )?,
            terminal_drains: Vec::new(),
            sequence_lanes,
        })
    }

    /// Returns the physical transport's immutable control surface.
    #[must_use]
    pub const fn transport(&self) -> &DocumentTransportSession {
        &self.transport
    }

    /// Pulls and admits at most one fairly selected logical message.
    pub async fn pump_next(
        &mut self,
        registry: &dyn AsyncMembershipRegistryPort,
    ) -> Result<Option<BufferDisposition>, AsyncTransportError> {
        if let Some(code) = self.pressure.closed_code() {
            self.transport.poll_bounded_close_once();
            return Ok(Some(BufferDisposition::Closed(code)));
        }
        let candidate_result = self.transport.next_delivery_candidate().await;
        for completed in std::mem::take(&mut self.transport.completed_drains) {
            if !self.terminal_drains.iter().any(|drain| {
                drain.subscription() == completed.subscription()
                    && drain.binding() == completed.binding()
            }) {
                self.terminal_drains.push(completed);
            }
        }
        let candidate = match candidate_result {
            Ok(Some(candidate)) => candidate,
            Ok(None) => {
                self.prune_empty_terminal_drains();
                self.purge_inactive_delivery();
                return Ok(None);
            }
            Err(error) => {
                self.prune_empty_terminal_drains();
                self.purge_inactive_delivery();
                return Err(error);
            }
        };
        let pressure_membership = PressureMembership::new(
            candidate.authorization.subscription().clone(),
            candidate.authorization.binding().clone(),
            candidate.authorization.document_scope().clone(),
            candidate
                .authorization
                .verified
                .claims()
                .authorization_memo()
                .clone(),
        );
        let pressure_position = candidate.envelope.position();
        let mut pulled_candidate = self
            .pressure
            .track_pulled_candidate(pressure_membership.clone(), pressure_position);
        let terminal = candidate.terminal;
        if terminal
            && !self.terminal_drains.iter().any(|drain| {
                drain.subscription() == candidate.authorization.subscription()
                    && drain.binding() == candidate.authorization.binding()
            })
        {
            self.terminal_drains.push(candidate.authorization.clone());
        }
        let now = candidate.authorization.authority.now();
        let mut authorized = match self.transport.seal_async_delivery(
            &candidate.authorization,
            candidate.envelope,
            registry,
            now,
            terminal,
            !terminal,
        ) {
            Ok(authorized) => authorized,
            Err(error) => {
                pulled_candidate.disarm();
                return Err(self.reject_pulled_delivery(
                    error,
                    pressure_membership.clone(),
                    pressure_position,
                ));
            }
        };
        let final_now = candidate.authorization.authority.now();
        if let Err(error) = self.transport.revalidate_async_delivery(
            &candidate.authorization,
            &mut authorized,
            registry,
            final_now,
            !terminal,
        ) {
            pulled_candidate.disarm();
            return Err(self.reject_pulled_delivery(
                error,
                pressure_membership.clone(),
                pressure_position,
            ));
        }
        let commit_now = candidate.authorization.authority.now();
        if let Err(error) = self.transport.revalidate_async_delivery(
            &candidate.authorization,
            &mut authorized,
            registry,
            commit_now,
            !terminal,
        ) {
            pulled_candidate.disarm();
            return Err(self.reject_pulled_delivery(
                error,
                pressure_membership.clone(),
                pressure_position,
            ));
        }
        if authorized.document_generation() != self.transport.control_generation {
            let error = AsyncTransportError::new(AsyncTransportErrorKind::StaleControl);
            return Err(self.reject_pulled_delivery(
                error,
                pressure_membership.clone(),
                pressure_position,
            ));
        }
        if !authorized.is_current_at(commit_now) {
            let error = AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost);
            return Err(self.reject_pulled_delivery(
                error,
                pressure_membership.clone(),
                pressure_position,
            ));
        }
        let disposition = match self.pressure.offer(authorized) {
            Ok(disposition) => disposition,
            Err(_) => {
                let error = AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope);
                pulled_candidate.disarm();
                return Err(self.reject_pulled_delivery(
                    error,
                    pressure_membership.clone(),
                    pressure_position,
                ));
            }
        };
        pulled_candidate.disarm();
        if matches!(disposition, BufferDisposition::Closed(_)) {
            self.transport.begin_bounded_close();
        }
        self.prune_empty_terminal_drains();
        Ok(Some(disposition))
    }

    /// Seals and atomically admits one complete replay for an exact active membership.
    pub fn admit_replay(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
        transcript: Vec<AsyncEnvelope>,
        registry: &dyn AsyncMembershipRegistryPort,
    ) -> Result<BufferDisposition, AsyncTransportError> {
        let result = self.admit_replay_inner(authorization, transcript, registry);
        if result.is_err() {
            self.pressure.record_replay_rejection();
        }
        result
    }

    fn admit_replay_inner(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
        transcript: Vec<AsyncEnvelope>,
        registry: &dyn AsyncMembershipRegistryPort,
    ) -> Result<BufferDisposition, AsyncTransportError> {
        if let Some(code) = self.pressure.closed_code() {
            self.transport.begin_bounded_close();
            return Ok(BufferDisposition::Closed(code));
        }
        match self.pressure.preflight_replay(&transcript) {
            ReplayPreflight::Ready => {}
            ReplayPreflight::Invalid => {
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::InvalidEnvelope,
                ));
            }
            ReplayPreflight::Closed(code) => {
                self.transport.begin_bounded_close();
                return Ok(BufferDisposition::Closed(code));
            }
        }
        let stored_authorization = self
            .transport
            .resolve_active_stored_authorization(authorization)?;
        for envelope in &transcript {
            stored_authorization
                .context()
                .validate_local_envelope(envelope)
                .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        }
        let replay_membership = PressureMembership::new(
            stored_authorization.subscription().clone(),
            stored_authorization.binding().clone(),
            stored_authorization.document_scope().clone(),
            stored_authorization
                .verified
                .claims()
                .authorization_memo()
                .clone(),
        );
        let lane_index = self
            .sequence_lanes
            .iter()
            .position(|lane| lane.matches(&stored_authorization))
            .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        let pressure_high_water = self.pressure.required_high_water(&replay_membership);
        let recovery_pending = self.sequence_lanes[lane_index].machine.state()
            == SequenceState::Degraded
            || pressure_high_water.is_some();
        if !recovery_pending {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        if transcript
            .iter()
            .any(|envelope| matches!(envelope.payload(), AsyncPayload::Complete(_)))
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        let replay_envelopes = transcript.iter().collect::<Vec<_>>();
        let pressure_recovery = pressure_high_water
            .filter(|_| self.sequence_lanes[lane_index].machine.state() == SequenceState::Current);
        self.sequence_lanes[lane_index]
            .machine
            .prepare_replay(&replay_envelopes, pressure_recovery)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        let commit_now = stored_authorization.authority.now();
        let sealed = self.transport.seal_async_replay(
            &stored_authorization,
            transcript,
            registry,
            commit_now,
        )?;
        if sealed.iter().any(|authorized| {
            authorized.document_generation() != self.transport.control_generation
                || !authorized.is_current_at(commit_now)
        }) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationLost,
            ));
        }
        let disposition = self
            .pressure
            .offer_replay(sealed)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        if matches!(disposition, BufferDisposition::Closed(_)) {
            self.transport.begin_bounded_close();
        }
        Ok(disposition)
    }

    /// Dispatches one leased queue head through Task 3 without exposing the lease.
    pub fn dispatch_next(
        &mut self,
        registry: &dyn AsyncMembershipRegistryPort,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
    ) -> Result<Option<AsyncDeliveryDisposition>, AsyncDeliveryError> {
        let Some(mut delivery) = self.pressure.try_start_delivery() else {
            return Ok(None);
        };
        if delivery.is_canceled() {
            self.prune_empty_terminal_drains();
            return Err(AsyncDeliveryError::new(AsyncDeliveryErrorKind::Retired));
        }

        let active = self
            .transport
            .memberships
            .iter()
            .find(|logical| {
                logical.authorization.subscription() == delivery.envelope().subscription()
                    && logical.authorization.binding() == delivery.binding()
            })
            .map(|logical| logical.authorization.clone());
        let terminal = self
            .terminal_drains
            .iter()
            .find(|authorization| {
                authorization.subscription() == delivery.envelope().subscription()
                    && authorization.binding() == delivery.binding()
            })
            .cloned();
        let (authorization, require_active) = match (active, terminal) {
            (Some(authorization), _) => (authorization, true),
            (None, Some(authorization)) => (authorization, false),
            (None, None) => {
                self.prune_empty_terminal_drains();
                return Err(AsyncDeliveryError::new(
                    AsyncDeliveryErrorKind::AuthorizationLost,
                ));
            }
        };
        let replay = delivery.is_replay();
        let mut dispatch_now = None;
        if !replay {
            let now = authorization.authority.now();
            if delivery.authorized_entries_mut().any(|authorized| {
                self.transport
                    .revalidate_async_delivery(
                        &authorization,
                        authorized,
                        registry,
                        now,
                        require_active,
                    )
                    .is_err()
                    || authorized.document_generation() != self.transport.control_generation
                    || !authorized.is_current_at(now)
            }) {
                self.prune_empty_terminal_drains();
                return Err(AsyncDeliveryError::new(
                    AsyncDeliveryErrorKind::AuthorizationLost,
                ));
            }
            dispatch_now = Some(now);
        }
        if delivery.is_canceled() {
            self.prune_empty_terminal_drains();
            return Err(AsyncDeliveryError::new(AsyncDeliveryErrorKind::Retired));
        }

        let pressure_membership = delivery.pressure_membership().clone();
        let Some(sequence) = self.sequence_lanes.iter_mut().find(|lane| {
            lane.subscription == *delivery.envelope().subscription()
                && lane.binding == *delivery.binding()
        }) else {
            self.prune_empty_terminal_drains();
            return Err(AsyncDeliveryError::new(
                AsyncDeliveryErrorKind::AuthorizationLost,
            ));
        };
        let outcome = if replay {
            let transport = &self.transport;
            let generation = transport.control_generation;
            delivery
                .dispatch_replay_with(&mut sequence.machine, dispatcher, |authorized| {
                    let now = authorization.authority.now();
                    if now >= authorization.verified.expires_at() {
                        return Err(LeaseDispatchError::MembershipExpired);
                    }
                    transport
                        .revalidate_async_delivery(
                            &authorization,
                            authorized,
                            registry,
                            now,
                            require_active,
                        )
                        .map_err(|_| LeaseDispatchError::AuthorizationLost)?;
                    if authorized.document_generation() != generation
                        || !authorized.is_current_at(now)
                    {
                        return Err(LeaseDispatchError::AuthorizationLost);
                    }
                    Ok(now)
                })
                .map(AsyncDeliveryDisposition::Replay)
        } else {
            delivery
                .dispatch(
                    &mut sequence.machine,
                    dispatch_now.expect("single delivery captured final current time"),
                    dispatcher,
                )
                .map(AsyncDeliveryDisposition::Sequence)
        };
        if let Ok(AsyncDeliveryDisposition::Replay(replay)) = &outcome {
            self.pressure
                .record_replay_recovery(&pressure_membership, replay.current());
        }
        self.pressure.commit_recoveries_if_drained();
        self.prune_empty_terminal_drains();
        match outcome {
            Ok(disposition) => Ok(Some(disposition)),
            Err(LeaseDispatchError::Retired) => {
                Err(AsyncDeliveryError::new(AsyncDeliveryErrorKind::Retired))
            }
            Err(LeaseDispatchError::AuthorizationLost) => Err(AsyncDeliveryError::new(
                AsyncDeliveryErrorKind::AuthorizationLost,
            )),
            Err(LeaseDispatchError::MembershipExpired) => Err(AsyncDeliveryError::new(
                AsyncDeliveryErrorKind::AuthorizationLost,
            )),
            Err(LeaseDispatchError::ReplayRetired(replay)) => Err(AsyncDeliveryError::with_replay(
                AsyncDeliveryErrorKind::Retired,
                replay,
            )),
            Err(LeaseDispatchError::ReplayAuthorizationLost(replay)) => Err(
                AsyncDeliveryError::with_replay(AsyncDeliveryErrorKind::AuthorizationLost, replay),
            ),
            Err(LeaseDispatchError::ReplayMembershipExpired(replay)) => Err(
                AsyncDeliveryError::with_replay(AsyncDeliveryErrorKind::AuthorizationLost, replay),
            ),
            Err(LeaseDispatchError::Sequence(error)) => Err(AsyncDeliveryError::new(
                AsyncDeliveryErrorKind::Sequence(error.kind()),
            )),
            Err(LeaseDispatchError::Replay(error)) => Err(AsyncDeliveryError::from_replay(error)),
        }
    }

    /// Returns the aggregate retained item count across every logical membership.
    #[must_use]
    pub fn retained_events(&self) -> usize {
        self.pressure.retained_events()
    }

    /// Returns aggregate canonical bytes retained for the whole document.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.pressure.retained_bytes()
    }

    /// Returns the number of active document delivery permits.
    #[must_use]
    pub fn active_permits(&self) -> usize {
        self.pressure.active_permits()
    }

    /// Returns whether queue or delivery loss requires continuity recovery.
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.pressure.is_degraded()
            || self
                .sequence_lanes
                .iter()
                .any(|lane| lane.machine.state() == SequenceState::Degraded)
    }

    /// Returns the bounded count of redacted unresolved pressure causes.
    #[must_use]
    pub fn unresolved_pressure_cause_count(&self) -> usize {
        self.pressure.unresolved_pressure_cause_count()
    }

    /// Returns one bounded low-cardinality pressure snapshot.
    #[must_use]
    pub const fn telemetry_snapshot(&self) -> AsyncTelemetrySnapshot {
        self.pressure.telemetry_snapshot()
    }

    /// Returns the Task 3 position owned by one exact logical membership.
    #[must_use]
    pub fn sequence_position(
        &self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Option<StreamPosition> {
        self.sequence_lanes
            .iter()
            .find(|lane| lane.matches(authorization))
            .map(|lane| lane.machine.current())
    }

    /// Returns the Task 3 continuity state owned by one exact logical membership.
    #[must_use]
    pub fn sequence_state(
        &self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Option<SequenceState> {
        self.sequence_lanes
            .iter()
            .find(|lane| lane.matches(authorization))
            .map(|lane| lane.machine.state())
    }

    /// Freshly authorizes and installs one host-authoritative continuity baseline.
    pub fn recover_from_authoritative_refresh(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
        registry: &dyn AsyncMembershipRegistryPort,
        authority: &dyn AsyncContinuityAuthorityPort,
    ) -> Result<BaselineDisposition, AsyncDeliveryError> {
        let stored_authorization = self
            .transport
            .resolve_active_stored_authorization(authorization)
            .map_err(|_| AsyncDeliveryError::new(AsyncDeliveryErrorKind::AuthorizationLost))?;
        let pressure_membership = PressureMembership::new(
            stored_authorization.subscription().clone(),
            stored_authorization.binding().clone(),
            stored_authorization.document_scope().clone(),
            stored_authorization
                .verified
                .claims()
                .authorization_memo()
                .clone(),
        );
        let pressure_high_water = self.pressure.required_high_water(&pressure_membership);
        let lane_index = self
            .sequence_lanes
            .iter()
            .position(|lane| lane.matches(&stored_authorization))
            .ok_or_else(|| AsyncDeliveryError::new(AsyncDeliveryErrorKind::AuthorizationLost))?;
        let baseline = authority
            .authoritative_refresh(
                self.sequence_lanes[lane_index]
                    .machine
                    .authoritative_refresh_request(pressure_high_water),
            )
            .ok_or_else(|| {
                AsyncDeliveryError::new(AsyncDeliveryErrorKind::Sequence(
                    SequenceErrorKind::AuthoritativeRefreshUnavailable,
                ))
            })?;
        let commit_now = stored_authorization.authority.now();
        stored_authorization
            .context()
            .validate_current_scope(
                stored_authorization.binding(),
                stored_authorization.document_scope(),
                registry,
                commit_now,
            )
            .map_err(|_| AsyncDeliveryError::new(AsyncDeliveryErrorKind::AuthorizationLost))?;
        let lane = &mut self.sequence_lanes[lane_index];
        let disposition = lane
            .machine
            .install_authoritative_baseline_covering(baseline, pressure_high_water)
            .map_err(|error| {
                AsyncDeliveryError::new(AsyncDeliveryErrorKind::Sequence(error.kind()))
            })?;
        self.pressure
            .record_replay_recovery(&pressure_membership, lane.machine.current());
        Ok(disposition)
    }

    /// Returns the bounded count of exact logical sequence lanes.
    #[must_use]
    pub fn delivery_lane_count(&self) -> usize {
        self.sequence_lanes.len()
    }

    /// Returns the bounded count of detached terminal drains.
    #[must_use]
    pub fn terminal_drain_count(&self) -> usize {
        self.terminal_drains.len()
    }

    /// Starts a typed add using the underlying Task 4 authority protocol.
    pub fn prepare_add(
        &self,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<PendingTransportAdd, AsyncTransportError> {
        self.transport.prepare_add(authorization)
    }

    /// Rechecks a freshly authorized add before source establishment.
    pub fn prepare_establish(
        &self,
        authorized: AuthorizedTransportAdd,
    ) -> Result<EstablishingTransportAdd, AsyncTransportError> {
        self.transport.prepare_establish(authorized)
    }

    /// Commits one established logical membership.
    pub fn commit_add(&mut self, ready: ReadyTransportAdd) -> Result<(), AsyncTransportError> {
        if let Some(existing) = self
            .terminal_drains
            .iter()
            .find(|drain| drain.subscription() == ready.authorization.subscription())
        {
            let kind = if existing.binding() == ready.authorization.binding() {
                AsyncTransportErrorKind::DuplicateMembership
            } else {
                AsyncTransportErrorKind::DescriptorMismatch
            };
            return Err(AsyncTransportError::new(kind));
        }
        let sequence = DocumentSequenceLane::new(&ready.authorization);
        self.transport.commit_add(ready)?;
        self.sequence_lanes.push(sequence);
        Ok(())
    }

    /// Starts a typed removal using the underlying Task 4 authority protocol.
    pub fn prepare_remove<'a>(
        &self,
        authorization: &'a AuthorizedTransportSubscription,
    ) -> Result<PendingTransportRemove<'a>, AsyncTransportError> {
        self.transport.prepare_remove(authorization)
    }

    /// Commits one freshly authorized logical removal.
    pub fn commit_remove(
        &mut self,
        ready: ReadyTransportRemove,
    ) -> Result<CloseDisposition, AsyncTransportError> {
        let subscription = ready.subscription.clone();
        let binding = ready.binding.clone();
        let disposition = self.transport.commit_remove(ready)?;
        if disposition == CloseDisposition::Closed {
            self.pressure.retire_membership(&subscription, &binding);
            self.sequence_lanes
                .retain(|lane| lane.subscription != subscription || lane.binding != binding);
        }
        Ok(disposition)
    }

    /// Retires aggregate delivery and closes every logical provider session.
    pub async fn close(&mut self) -> Result<CloseDisposition, AsyncTransportError> {
        self.terminal_drains.clear();
        self.sequence_lanes.clear();
        self.pressure.retire();
        self.transport.close().await
    }

    /// Retires and drains the aggregate delivery queue exactly once.
    pub fn retire_delivery(&mut self) -> Retirement {
        self.terminal_drains.clear();
        self.sequence_lanes.clear();
        self.pressure.retire()
    }

    fn purge_inactive_delivery(&mut self) {
        let memberships = &self.transport.memberships;
        let terminal_drains = &self.terminal_drains;
        self.pressure
            .retain_current_memberships(|subscription, binding| {
                memberships.iter().any(|logical| {
                    logical.authorization.subscription() == subscription
                        && logical.authorization.binding() == binding
                }) || terminal_drains.iter().any(|authorization| {
                    authorization.subscription() == subscription
                        && authorization.binding() == binding
                })
            });
        self.prune_sequence_lanes();
    }

    fn reject_pulled_delivery(
        &mut self,
        error: AsyncTransportError,
        membership: PressureMembership,
        high_water: StreamPosition,
    ) -> AsyncTransportError {
        self.pressure.record_delivery_loss(membership, high_water);
        self.prune_empty_terminal_drains();
        error
    }

    fn prune_empty_terminal_drains(&mut self) {
        let drains = std::mem::take(&mut self.terminal_drains);
        self.terminal_drains = drains
            .into_iter()
            .filter(|authorization| {
                self.pressure
                    .has_membership_entries(authorization.subscription(), authorization.binding())
            })
            .collect();
        self.prune_sequence_lanes();
    }

    fn prune_sequence_lanes(&mut self) {
        let memberships = &self.transport.memberships;
        let terminal_drains = &self.terminal_drains;
        self.sequence_lanes.retain(|lane| {
            memberships.iter().any(|logical| {
                logical.authorization.subscription() == &lane.subscription
                    && logical.authorization.binding() == &lane.binding
            }) || terminal_drains.iter().any(|authorization| {
                authorization.subscription() == &lane.subscription
                    && authorization.binding() == &lane.binding
            })
        });
    }
}

impl fmt::Debug for BoundedDocumentTransportSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedDocumentTransportSession")
            .field("transport", &self.transport)
            .field("pressure", &self.pressure)
            .finish()
    }
}

fn invalid_origin() -> AsyncTransportError {
    AsyncTransportError::new(AsyncTransportErrorKind::InvalidOrigin)
}

fn hash_document_scope_part(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn hash_optional_document_scope_part(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            digest.update([1]);
            hash_document_scope_part(digest, value);
        }
        None => digest.update([0]),
    }
}

fn valid_origin_host(host: &str) -> bool {
    if host.starts_with('[') {
        let Some(inner) = host
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        else {
            return false;
        };
        return !inner.contains('%') && std::net::Ipv6Addr::from_str(inner).is_ok();
    }
    if host.is_empty() || host.len() > 253 || host.ends_with('.') {
        return false;
    }
    if std::net::Ipv4Addr::from_str(host).is_ok() {
        return true;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}
