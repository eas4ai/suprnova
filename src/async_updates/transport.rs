//! Host-neutral logical subscription and document transport contracts.

use std::error::Error;
use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::str::FromStr;
use std::task::Poll;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::identity::UnixMillis;

use super::{
    AsyncEnvelope, AsyncEnvelopeContext, AsyncMembershipRegistryPort, AuthorizationMemo,
    AuthorizedSubscription, StreamPosition, SubscriptionId, VerifiedSubscriptionDescriptor,
};

const MIN_DOCUMENT_HANDLE_BYTES: usize = 16;
const MAX_DOCUMENT_HANDLE_BYTES: usize = 32;

/// Maximum logical subscriptions retained by one document transport.
pub const MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS: usize = 128;

/// Executor-neutral future returned by asynchronous transport ports.
pub type AsyncTransportFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result of closing one logical or document transport session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseDisposition {
    /// This call performed the close transition.
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

/// Descriptor-bound authorization required to add or remove one logical membership.
pub struct AuthorizedTransportSubscription {
    context: AsyncEnvelopeContext,
    verified: VerifiedSubscriptionDescriptor,
    origin: VerifiedOrigin,
}

impl AuthorizedTransportSubscription {
    /// Binds Task 2 authorization to one active logical membership and exact origin.
    pub fn new(
        authorized: &AuthorizedSubscription,
        subscription: SubscriptionId,
        registry: &dyn AsyncMembershipRegistryPort,
        origin: VerifiedOrigin,
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
            origin,
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

    fn authorization_memo(&self) -> &AuthorizationMemo {
        self.verified.claims().authorization_memo()
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
    ) -> AsyncTransportFuture<'a, Result<Box<dyn AsyncEventSession>, AsyncTransportError>>;
}

/// Host-neutral session for one currently authorized logical subscription.
///
/// `next` must be cancellation-safe: dropping a pending future may not consume
/// or reorder a message. This permits bounded document fan-in without binding
/// the engine to an executor or spawning detached tasks. Implementations must
/// also release provider resources when dropped; `close` is the explicit
/// graceful-shutdown path, not the only cleanup safety net.
pub trait AsyncEventSession: Send {
    /// Returns the exact authoritative baseline bound to this logical session.
    fn baseline(&self) -> StreamPosition;

    /// Returns the next bounded authorized envelope, or `None` after completion.
    fn next<'a>(
        &'a mut self,
    ) -> AsyncTransportFuture<'a, Result<Option<AsyncEnvelope>, AsyncTransportError>>;

    /// Closes this logical session idempotently.
    ///
    /// Like `next`, this operation must be cancellation-safe. A caller may
    /// drop the returned future and invoke `close` again during controlled
    /// shutdown without losing the logical session's cleanup authority.
    fn close<'a>(
        &'a mut self,
    ) -> AsyncTransportFuture<'a, Result<CloseDisposition, AsyncTransportError>>;
}

struct LogicalTransportSession {
    authorization: VerifiedSubscriptionDescriptor,
    subscription: SubscriptionId,
    session: Box<dyn AsyncEventSession>,
    retiring: bool,
}

/// Bounded fan-in owner for compatible logical subscriptions.
pub struct DocumentTransportSession {
    origin: VerifiedOrigin,
    kind: DocumentTransportKind,
    handle: DocumentTransportHandle,
    limits: DocumentTransportLimits,
    authorization_scope: Option<AuthorizationMemo>,
    memberships: Vec<LogicalTransportSession>,
    cursor: usize,
    closing: bool,
    closed: bool,
}

impl DocumentTransportSession {
    /// Creates one physical transport owner with no active logical memberships.
    #[must_use]
    pub const fn new(
        origin: VerifiedOrigin,
        kind: DocumentTransportKind,
        handle: DocumentTransportHandle,
        limits: DocumentTransportLimits,
    ) -> Self {
        Self {
            origin,
            kind,
            handle,
            limits,
            authorization_scope: None,
            memberships: Vec::new(),
            cursor: 0,
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

    /// Returns whether one logical subscription is currently multiplexed here.
    #[must_use]
    pub fn contains_membership(&self, subscription: &SubscriptionId) -> bool {
        self.memberships
            .iter()
            .any(|logical| &logical.subscription == subscription)
    }

    /// Adds one currently authorized descriptor-bound logical session.
    pub async fn add(
        &mut self,
        source: &dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<(), AsyncTransportError> {
        self.validate_add(&authorization)?;
        let mut session = source.subscribe(&authorization).await?;
        if session.baseline() != authorization.baseline() {
            let _ = session.close().await;
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::BaselineMismatch,
            ));
        }
        if self.authorization_scope.is_none() {
            self.authorization_scope = Some(authorization.authorization_memo().clone());
        }
        self.memberships.push(LogicalTransportSession {
            authorization: authorization.verified,
            subscription: authorization.context.subscription().clone(),
            session,
            retiring: false,
        });
        Ok(())
    }

    /// Removes and closes one membership only with matching current authorization.
    pub async fn remove(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<CloseDisposition, AsyncTransportError> {
        self.validate_common(authorization)?;
        let Some(index) = self
            .memberships
            .iter()
            .position(|logical| &logical.subscription == authorization.subscription())
        else {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::UnknownMembership,
            ));
        };
        if self.memberships[index].authorization != authorization.verified {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::DescriptorMismatch,
            ));
        }
        self.memberships[index].retiring = true;
        let disposition = self.memberships[index].session.close().await?;
        self.memberships.remove(index);
        self.repair_cursor_after_removal(index);
        Ok(disposition)
    }

    /// Waits for the next envelope from any logical session in bounded round-robin order.
    pub async fn next(&mut self) -> Result<Option<AsyncEnvelope>, AsyncTransportError> {
        if self.closed || self.closing {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        loop {
            if self.memberships.is_empty() {
                return Ok(None);
            }
            if let Some(index) = self.memberships.iter().position(|logical| logical.retiring) {
                self.memberships[index].session.close().await?;
                self.memberships.remove(index);
                self.repair_cursor_after_removal(index);
                continue;
            }
            let (index, result) = poll_fn(|task| {
                let count = self.memberships.len();
                for offset in 0..count {
                    let index = (self.cursor + offset) % count;
                    let polled = {
                        let mut next = self.memberships[index].session.next();
                        next.as_mut().poll(task)
                    };
                    if let Poll::Ready(result) = polled {
                        return Poll::Ready((index, result));
                    }
                }
                Poll::Pending
            })
            .await;
            match result {
                Err(error) => {
                    self.memberships[index].retiring = true;
                    if self.memberships[index].session.close().await.is_ok() {
                        self.memberships.remove(index);
                        self.repair_cursor_after_removal(index);
                    }
                    return Err(error);
                }
                Ok(Some(envelope)) => {
                    if envelope.subscription() != &self.memberships[index].subscription {
                        self.memberships[index].retiring = true;
                        if self.memberships[index].session.close().await.is_ok() {
                            self.memberships.remove(index);
                            self.repair_cursor_after_removal(index);
                        }
                        return Err(AsyncTransportError::new(
                            AsyncTransportErrorKind::RoutingMismatch,
                        ));
                    }
                    self.cursor = (index + 1) % self.memberships.len();
                    return Ok(Some(envelope));
                }
                Ok(None) => {
                    self.memberships[index].retiring = true;
                    self.memberships[index].session.close().await?;
                    self.memberships.remove(index);
                    self.repair_cursor_after_removal(index);
                }
            }
        }
    }

    /// Closes every logical session and the document owner exactly once.
    pub async fn close(&mut self) -> Result<CloseDisposition, AsyncTransportError> {
        if self.closed {
            return Ok(CloseDisposition::AlreadyClosed);
        }
        self.closing = true;
        let mut first_error = None;
        let mut index = self.memberships.len();
        while index > 0 {
            index -= 1;
            match self.memberships[index].session.close().await {
                Ok(_) => {
                    self.memberships.remove(index);
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        self.repair_cursor();
        self.closed = self.memberships.is_empty();
        self.closing = !self.closed;
        match first_error {
            Some(error) => Err(error),
            None => Ok(CloseDisposition::Closed),
        }
    }

    fn validate_add(
        &self,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<(), AsyncTransportError> {
        self.validate_common(authorization)?;
        if let Some(existing) = self
            .memberships
            .iter()
            .find(|logical| &logical.subscription == authorization.subscription())
        {
            let kind = if existing.authorization == authorization.verified {
                AsyncTransportErrorKind::DuplicateMembership
            } else {
                AsyncTransportErrorKind::DescriptorMismatch
            };
            return Err(AsyncTransportError::new(kind));
        }
        if self.memberships.len() == self.limits.max_memberships() {
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
        if self.closed || self.closing {
            return Err(AsyncTransportError::new(AsyncTransportErrorKind::Closed));
        }
        if authorization.origin() != &self.origin {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::OriginMismatch,
            ));
        }
        if self
            .authorization_scope
            .as_ref()
            .is_some_and(|scope| scope != authorization.authorization_memo())
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::AuthorizationScopeMismatch,
            ));
        }
        Ok(())
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
}

impl fmt::Debug for DocumentTransportSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentTransportSession")
            .field("origin", &self.origin)
            .field("kind", &self.kind)
            .field("handle", &self.handle)
            .field("membership_count", &self.memberships.len())
            .field("max_memberships", &self.limits.max_memberships())
            .field("closing", &self.closing)
            .field("closed", &self.closed)
            .finish()
    }
}

fn invalid_origin() -> AsyncTransportError {
    AsyncTransportError::new(AsyncTransportErrorKind::InvalidOrigin)
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
