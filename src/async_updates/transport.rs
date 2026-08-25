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

use super::{
    AsyncEnvelope, AsyncEnvelopeContext, AsyncMembershipRegistryPort, AsyncPayload,
    AuthorizationMemo, AuthorizedSubscription, BoundedEventContracts, BoundedTopics, StreamName,
    StreamPosition, SubscriptionBinding, SubscriptionId, SubscriptionMode, SubscriptionModes,
    VerifiedSubscriptionDescriptor,
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
            subscription: self.authorization.subscription().clone(),
            binding: self.authorization.binding.clone(),
            expires_at: self.authorization.verified.expires_at(),
            authority: self.authorization.authority.clone(),
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
    subscription: SubscriptionId,
    binding: SubscriptionBinding,
    expires_at: UnixMillis,
    authority: Arc<dyn AsyncTransportAuthorityPort>,
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
    authorization: SubscriptionBinding,
    subscription: SubscriptionId,
    session: Pin<Box<dyn AsyncEventSession>>,
}

struct RetiringTransportSession {
    authorization: SubscriptionBinding,
    subscription: SubscriptionId,
    session: Pin<Box<dyn AsyncEventSession>>,
}

enum DocumentPoll {
    Envelope(AsyncEnvelope),
    Retired,
    Empty,
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
            .any(|logical| &logical.subscription == subscription)
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
        self.validate_ready_control(&ready.document, ready.expires_at, ready.authority.as_ref())?;
        self.validate_ready_add(&ready.subscription, &ready.binding)?;
        self.ensure_generation_available()?;
        let _permit = ready.permit;
        let session = ready
            .session
            .take()
            .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed))?;
        self.memberships.push(LogicalTransportSession {
            authorization: ready.binding,
            subscription: ready.subscription,
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
            .position(|logical| logical.subscription == ready.subscription)
        else {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::UnknownMembership,
            ));
        };
        if self.memberships[index].authorization != ready.binding {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::DescriptorMismatch,
            ));
        }
        self.ensure_generation_available()?;
        let retirement = self.detach_membership(index);
        self.poll_exact_retirement_once(retirement);
        Ok(CloseDisposition::Closed)
    }

    /// Waits for the next envelope from any logical session in bounded round-robin order.
    pub async fn next(&mut self) -> Result<Option<AsyncEnvelope>, AsyncTransportError> {
        if self.closed || self.closing {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        loop {
            match poll_fn(|task| self.poll_document_next(task)).await? {
                DocumentPoll::Envelope(envelope) => return Ok(Some(envelope)),
                DocumentPoll::Retired => {}
                DocumentPoll::Empty => return Ok(None),
            }
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
        let active = self
            .memberships
            .iter()
            .map(|logical| (&logical.subscription, &logical.authorization));
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
            authorization: logical.authorization,
            subscription: logical.subscription,
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
                    let retirement = self.detach_membership(index);
                    self.poll_retirement_at(retirement, task);
                    return Poll::Ready(Ok(DocumentPoll::Retired));
                }
                Ok(Some(envelope)) => {
                    if envelope.subscription() != &self.memberships[index].subscription {
                        let retirement = self.detach_membership(index);
                        self.poll_retirement_at(retirement, task);
                        return Poll::Ready(Err(AsyncTransportError::new(
                            AsyncTransportErrorKind::RoutingMismatch,
                        )));
                    }
                    if matches!(envelope.payload(), AsyncPayload::Complete(_)) {
                        let retirement = self.detach_membership(index);
                        self.poll_retirement_at(retirement, task);
                    } else {
                        self.cursor = (index + 1) % self.memberships.len();
                    }
                    return Poll::Ready(Ok(DocumentPoll::Envelope(envelope)));
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
