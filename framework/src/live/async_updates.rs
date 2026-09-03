//! Framework-owned runtime for authorized asynchronous updates.
//!
//! The runtime issues signed subscriptions through the engine's subscription
//! service, keeps one bounded per-subscription log of typed envelopes, owns the
//! physical SSE and WebSocket document transports, and drives the engine's
//! bounded document delivery. Application code publishes only through
//! [`super::LiveStreams`]; browser input never creates authority here.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::num::{NonZeroU16, NonZeroUsize};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use futures::FutureExt as _;
use sha2::{Digest, Sha256};
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncContinuityAuthorityPort, AsyncContinuityRequest,
    AsyncDeliveryDisposition, AsyncDeliveryErrorKind, AsyncDispatchError, AsyncEnvelope,
    AsyncEnvelopeContext, AsyncEnvelopeDispatchPort, AsyncEventSession, AsyncEventSource,
    AsyncMembershipRegistryPort, AsyncMembershipRequest, AsyncMembershipValidation, AsyncPayload,
    AsyncPolicy, AsyncTransportAuthorityPort, AsyncTransportAuthorityRequest,
    AsyncTransportAuthorityValidation, AsyncTransportError, AsyncTransportErrorKind,
    AsyncTransportFuture, AuthorizedSubscription, AuthorizedTransportSubscription,
    BoundedDocumentTransportSession, BoundedPresentationSignalContracts, BufferDisposition,
    CapabilityVersion, CloseDisposition, CurrentSubscriptionRegistration,
    DocumentAuthorizationScope, DocumentTransportHandle, DocumentTransportKind,
    DocumentTransportLimits, DocumentTransportSession, EventTarget, Heartbeat,
    MAX_ASYNC_BUFFER_BYTES, MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_PAYLOAD_BYTES,
    MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS, MAX_EVENT_FANOUT, MAX_REPLAY_TRANSCRIPT_ENVELOPES,
    PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy, RegisteredBrowserEvent,
    RegisteredRefresh, ResolvedAsyncDelivery, ResolvedEventFanout, SequenceDisposition, SseEncoder,
    SseMembershipControl, StreamEpoch, StreamName, StreamPosition, StreamSequence,
    SubscriptionBinding, SubscriptionDescriptor, SubscriptionError, SubscriptionErrorKind,
    SubscriptionId, SubscriptionIssueRequest, SubscriptionModes, SubscriptionService, TopicName,
    TrustedMountParameters, VerifiedOrigin, WebSocketCodec, WebSocketControlRecord,
    WebSocketMembershipAcknowledgment, WebSocketMembershipControl, WebSocketMembershipRequest,
    encode_async_envelope,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock;
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::host::{HostScopeFacts, TrustedLiveRequestContext};
use suprnova_live::identity::{
    BrowserOperationName, ComponentName, ContentDigest, IslandSlot, UnixMillis,
};
use suprnova_live::registry::ComponentRegistry;
use suprnova_live::resource::{PermitPool, ResourceBounds};
use tokio::sync::{Notify, mpsc};

use super::streams::LiveEventTarget;
use crate::FrameworkError;

/// Reserved versioned control path for issuing and renewing subscriptions.
pub(crate) const LIVE_ASYNC_SUBSCRIPTION_PATH: &str = "/__live/v1/async/subscriptions";
/// Reserved versioned control path for SSE membership changes.
pub(crate) const LIVE_ASYNC_MEMBERSHIP_PATH: &str = "/__live/v1/async/memberships";
/// Reserved versioned Server-Sent Events stream path.
pub(crate) const LIVE_ASYNC_EVENTS_PATH: &str = "/__live/v1/async/events";
/// Reserved versioned WebSocket transport path.
pub(crate) const LIVE_ASYNC_SOCKET_PATH: &str = "/__live/v1/async/socket";

pub(crate) const SUBSCRIPTION_LIFETIME_MS: u64 = 120_000;
pub(crate) const HEARTBEAT_TIMEOUT_MS: u64 = 15_000;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Delay after a productive SSE batch before a non-authoritative comment
/// follows it. WebKit hands a fetch stream's buffered bytes to the page only
/// when further bytes arrive, so without a trailer the tail of a batch stays
/// invisible until the next idle heartbeat.
const SSE_DELIVERY_TRAILER_DELAY: Duration = Duration::from_millis(200);
pub(crate) const POLL_INTERVAL_MS: u64 = 30_000;
pub(crate) const POLL_JITTER_BASIS_POINTS: u16 = 2_000;
pub(crate) const RECONNECT_MINIMUM_DELAY_MS: u64 = 250;
pub(crate) const RECONNECT_MAXIMUM_DELAY_MS: u64 = 5_000;
pub(crate) const DEFAULT_RECONNECT_ATTEMPTS: u8 = 4;
pub(crate) const MIN_DOCUMENT_INSTANCE_BYTES: usize = 16;
pub(crate) const MAX_DOCUMENT_INSTANCE_BYTES: usize = 64;
pub(crate) const MAX_CONTROL_NONCE_BYTES: usize = 128;
const MAX_TRANSPORTS_PER_SCOPE: usize = 8;
/// Process-wide bound on live document transports across every scope, so
/// rotating sessions cannot grow the transport table without limit.
const MAX_TRANSPORTS_TOTAL: usize = 4_096;
const MAX_ISSUED_PER_SCOPE: usize = 512;
const MAX_LOG_ENTRIES: usize = 256;
const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_REMEMBERED_NONCES: usize = 256;
pub(crate) const MAX_SOCKET_CONTROLS: u32 = 64;
const OUTBOUND_CAPACITY: usize = 16;
const DELIVERY_BATCH: usize = 32;
const MAX_BROWSER_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const TRANSPORT_POLICY_PURPOSE: &[u8] = b"suprnova-live/framework-transport-policy/v1\0";
const TARGET_SCOPE_PURPOSE: &[u8] = b"suprnova-live/framework-target-scope/v1\0";

/// Closed failure classes for the asynchronous control and transport routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AsyncErrorKind {
    ProtocolInvalid,
    TransportInvalid,
    TransportUnsupported,
    TransportMismatch,
    MountUnknown,
    StreamUnknown,
    SubscriptionUnknown,
    ContextRejected,
    AuthorizationDenied,
    AuthorityMissing,
    AuthorityInvalid,
    AuthorityExpired,
    GenerationInvalid,
    GenerationStale,
    TransportClosed,
    TransportReaderExists,
    TransportLimit,
    SubscriptionLimit,
    MembershipInvalid,
    MembershipUnknown,
    MembershipDuplicate,
    MembershipLimit,
    ControlInFlight,
    ControlCapacityExceeded,
    ControlReplayed,
    PositionInvalid,
    Unavailable,
}

impl AsyncErrorKind {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::ProtocolInvalid => "async_protocol_invalid",
            Self::TransportInvalid => "async_transport_invalid",
            Self::TransportUnsupported => "async_transport_unsupported",
            Self::TransportMismatch => "async_transport_mismatch",
            Self::MountUnknown => "async_mount_unknown",
            Self::StreamUnknown => "async_stream_unknown",
            Self::SubscriptionUnknown => "async_subscription_unknown",
            Self::ContextRejected => "async_context_rejected",
            Self::AuthorizationDenied => "async_authorization_denied",
            Self::AuthorityMissing => "async_authority_missing",
            Self::AuthorityInvalid => "async_authority_invalid",
            Self::AuthorityExpired => "async_authority_expired",
            Self::GenerationInvalid => "async_generation_invalid",
            Self::GenerationStale => "async_generation_stale",
            Self::TransportClosed => "async_transport_closed",
            Self::TransportReaderExists => "async_transport_reader_exists",
            Self::TransportLimit => "async_transport_limit",
            Self::SubscriptionLimit => "async_subscription_limit",
            Self::MembershipInvalid => "async_membership_invalid",
            Self::MembershipUnknown => "async_membership_unknown",
            Self::MembershipDuplicate => "async_membership_duplicate",
            Self::MembershipLimit => "async_membership_limit",
            Self::ControlInFlight => "async_control_in_flight",
            Self::ControlCapacityExceeded => "async_control_capacity_exceeded",
            Self::ControlReplayed => "async_control_replayed",
            Self::PositionInvalid => "async_position_invalid",
            Self::Unavailable => "async_unavailable",
        }
    }

    pub(crate) const fn status(self) -> u16 {
        match self {
            Self::ProtocolInvalid
            | Self::TransportInvalid
            | Self::TransportUnsupported
            | Self::TransportMismatch
            | Self::GenerationInvalid
            | Self::PositionInvalid => 400,
            Self::AuthorityMissing => 401,
            Self::ContextRejected
            | Self::AuthorizationDenied
            | Self::AuthorityInvalid
            | Self::MembershipInvalid => 403,
            Self::MountUnknown
            | Self::StreamUnknown
            | Self::SubscriptionUnknown
            | Self::MembershipUnknown => 404,
            Self::GenerationStale
            | Self::TransportClosed
            | Self::TransportReaderExists
            | Self::TransportLimit
            | Self::SubscriptionLimit
            | Self::MembershipDuplicate
            | Self::MembershipLimit
            | Self::ControlInFlight
            | Self::ControlCapacityExceeded
            | Self::ControlReplayed => 409,
            Self::AuthorityExpired => 410,
            Self::Unavailable => 503,
        }
    }

    /// Returns the bounded WebSocket close reason for one control failure.
    pub(crate) const fn socket_reason(self) -> &'static str {
        match self {
            Self::MembershipLimit => "membership_limit",
            Self::MembershipDuplicate => "membership_duplicate",
            Self::MembershipUnknown => "membership_unknown",
            Self::ControlInFlight => "control_in_flight",
            Self::ControlCapacityExceeded => "control_capacity_exceeded",
            Self::Unavailable => "unavailable",
            _ => "membership_authority_invalid",
        }
    }
}

/// Typed publication requested by application code.
#[derive(Clone, Debug)]
pub(crate) enum StreamPayloadSpec {
    Refresh,
    BrowserEvent {
        name: String,
        version: u16,
        target: LiveEventTarget,
        payload: CanonicalValue,
    },
}

/// Closed publication failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublishError {
    InvalidTopic,
    InvalidPayload,
}

/// Exact browser-facing transport kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransportKind {
    Sse,
    WebSocket,
}

impl TransportKind {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "sse" => Some(Self::Sse),
            "websocket" => Some(Self::WebSocket),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Sse => "sse",
            Self::WebSocket => "websocket",
        }
    }

    pub(crate) const fn document_kind(self) -> DocumentTransportKind {
        match self {
            Self::Sse => DocumentTransportKind::ServerSentEvents,
            Self::WebSocket => DocumentTransportKind::WebSocket,
        }
    }

    const fn mode(self) -> suprnova_live::async_updates::SubscriptionMode {
        match self {
            Self::Sse => suprnova_live::async_updates::SubscriptionMode::ServerSentEvents,
            Self::WebSocket => suprnova_live::async_updates::SubscriptionMode::WebSocket,
        }
    }
}

/// One engine document session behind its own asynchronous lock.
///
/// Engine document calls invoke the host ports, which read the tables, so a
/// document is never touched while the tables mutex is held.
type DocumentSlot = Arc<tokio::sync::Mutex<Option<BoundedDocumentTransportSession>>>;

/// Identity of one physical document transport owned by this process.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TransportKey {
    scope: String,
    kind: TransportKind,
    instance: String,
}

struct LogEntry {
    sequence: u64,
    envelope: AsyncEnvelope,
    encoded: Bytes,
}

/// Bounded ordered log of typed envelopes for one logical subscription.
pub(crate) struct SubscriptionLog {
    epoch: u64,
    next_sequence: u64,
    entries: VecDeque<LogEntry>,
    bytes: usize,
    waker: Option<Waker>,
    last_append_ms: u64,
}

impl SubscriptionLog {
    fn new(epoch: u64, now_ms: u64) -> Self {
        Self {
            epoch,
            next_sequence: 1,
            entries: VecDeque::new(),
            bytes: 0,
            waker: None,
            last_append_ms: now_ms,
        }
    }

    const fn head(&self) -> u64 {
        self.next_sequence - 1
    }

    fn oldest(&self) -> Option<u64> {
        self.entries.front().map(|entry| entry.sequence)
    }

    fn next_position(&self) -> StreamPosition {
        StreamPosition::new(
            StreamEpoch::new(self.epoch),
            StreamSequence::new(self.next_sequence),
        )
    }

    fn append(&mut self, envelope: AsyncEnvelope, encoded: Bytes, now_ms: u64) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.bytes = self.bytes.saturating_add(encoded.len());
        self.entries.push_back(LogEntry {
            sequence,
            envelope,
            encoded,
        });
        while self.entries.len() > MAX_LOG_ENTRIES || self.bytes > MAX_LOG_BYTES {
            if let Some(evicted) = self.entries.pop_front() {
                self.bytes = self.bytes.saturating_sub(evicted.encoded.len());
            } else {
                break;
            }
        }
        self.last_append_ms = now_ms;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn entry_at(&self, sequence: u64) -> Option<&LogEntry> {
        let first = self.oldest()?;
        if sequence < first {
            return None;
        }
        let index = usize::try_from(sequence - first).ok()?;
        self.entries
            .get(index)
            .filter(|entry| entry.sequence == sequence)
    }

    /// Returns every retained envelope after `position`, or `None` when the tail was evicted.
    fn tail_after(&self, epoch: u64, sequence: u64) -> Option<Vec<Bytes>> {
        if epoch != self.epoch || sequence > self.head() {
            return None;
        }
        if sequence == self.head() {
            return Some(Vec::new());
        }
        let first_needed = sequence.saturating_add(1);
        let oldest = self.oldest()?;
        if oldest > first_needed {
            return None;
        }
        Some(
            self.entries
                .iter()
                .filter(|entry| entry.sequence >= first_needed)
                .map(|entry| entry.encoded.clone())
                .collect(),
        )
    }
}

struct LogSession {
    log: Arc<Mutex<SubscriptionLog>>,
    baseline: StreamPosition,
    cursor: u64,
    delivery_cursor: Arc<AtomicU64>,
    closed: bool,
}

impl AsyncEventSession for LogSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn poll_next(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(None));
        }
        let mut log = lock_log(&this.log);
        if let Some(oldest) = log.oldest()
            && oldest > this.cursor
        {
            // Evicted entries can only be recovered by the browser's own
            // replay from its committed position; the lane observes the jump.
            this.cursor = oldest;
        }
        if let Some(entry) = log.entry_at(this.cursor) {
            let envelope = entry.envelope.clone();
            this.cursor = this.cursor.saturating_add(1);
            this.delivery_cursor.store(this.cursor, Ordering::Release);
            return Poll::Ready(Ok(Some(envelope)));
        }
        log.waker = Some(task.waker().clone());
        Poll::Pending
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>> {
        if self.closed {
            Poll::Ready(Ok(CloseDisposition::AlreadyClosed))
        } else {
            self.closed = true;
            Poll::Ready(Ok(CloseDisposition::Closed))
        }
    }
}

/// One issued logical subscription with its retained authority and log.
pub(crate) struct IssuedRecord {
    subscription: SubscriptionId,
    pub(crate) descriptor: SubscriptionDescriptor,
    pub(crate) binding: SubscriptionBinding,
    pub(crate) binding_text: String,
    previous_binding: Option<SubscriptionBinding>,
    authorized: Arc<AuthorizedSubscription>,
    context: Option<AsyncEnvelopeContext>,
    pub(crate) component: ComponentName,
    contract: ContentDigest,
    pub(crate) stream: StreamName,
    parameters: TrustedMountParameters,
    pub(crate) document_scope: DocumentAuthorizationScope,
    pub(crate) kind: TransportKind,
    pub(crate) transport: TransportKey,
    modes: SubscriptionModes,
    topics: Vec<String>,
    log: Arc<Mutex<SubscriptionLog>>,
    delivery_cursor: Arc<AtomicU64>,
    resume_position: u64,
    pub(crate) expires_at: UnixMillis,
    membership: Option<AuthorizedTransportSubscription>,
    control_in_flight: bool,
    transport_wake: Option<Arc<Notify>>,
}

impl IssuedRecord {
    fn context(&self) -> Option<&AsyncEnvelopeContext> {
        self.context.as_ref()
    }

    fn binding_matches(&self, binding: &SubscriptionBinding) -> bool {
        &self.binding == binding || self.previous_binding.as_ref() == Some(binding)
    }
}

/// One physical SSE or WebSocket document transport.
pub(crate) struct TransportRecord {
    pub(crate) kind: TransportKind,
    scope: DocumentAuthorizationScope,
    pub(crate) origin: VerifiedOrigin,
    pub(crate) credential: Option<String>,
    pub(crate) credential_expires_at: UnixMillis,
    handle: DocumentTransportHandle,
    document: DocumentSlot,
    retained_events: usize,
    retained_bytes: usize,
    degraded: bool,
    pub(crate) generation: u64,
    pub(crate) reader_active: bool,
    outbound: Option<mpsc::Sender<Bytes>>,
    wake: Arc<Notify>,
    closed: Arc<Notify>,
    used_nonces: VecDeque<String>,
    memberships: BTreeSet<String>,
    pub(crate) controls_used: u32,
    coalesced: u64,
    degraded_lanes: u64,
}

impl TransportRecord {
    fn remember_nonce(&mut self, nonce: &str) -> bool {
        if self.used_nonces.iter().any(|known| known == nonce) {
            return false;
        }
        if self.used_nonces.len() >= MAX_REMEMBERED_NONCES {
            self.used_nonces.pop_front();
        }
        self.used_nonces.push_back(nonce.to_owned());
        true
    }
}

#[derive(Default)]
pub(crate) struct AsyncTables {
    issued: HashMap<String, IssuedRecord>,
    transports: HashMap<TransportKey, TransportRecord>,
    credentials: HashMap<String, TransportKey>,
    topics: HashMap<String, BTreeSet<String>>,
}

struct ConstructingClaims {
    subscription: String,
    stream: StreamName,
    events: suprnova_live::async_updates::BoundedEventContracts,
}

/// Shared asynchronous-update state behind the immutable runtime graph.
pub(crate) struct AsyncState {
    tables: Mutex<AsyncTables>,
    constructing: Mutex<Option<ConstructingClaims>>,
    service: SubscriptionService,
    clock: Arc<dyn Clock>,
    engine_registry: Arc<ComponentRegistry>,
    membership_registry: Arc<MembershipRegistryPort>,
    transport_authority: Arc<TransportAuthorityPort>,
    source: LogEventSource,
    retirement: Notify,
    signals: BoundedPresentationSignalContracts,
}

impl AsyncState {
    pub(crate) fn new(
        keys: SnapshotKeyRing,
        clock: Arc<dyn Clock>,
        engine_registry: Arc<ComponentRegistry>,
    ) -> Result<Arc<Self>, FrameworkError> {
        let signals = BoundedPresentationSignalContracts::new(Vec::new())
            .map_err(|_| FrameworkError::internal("Live async signal contracts were rejected"))?;
        Ok(Arc::new_cyclic(|weak: &Weak<Self>| Self {
            tables: Mutex::new(AsyncTables::default()),
            constructing: Mutex::new(None),
            service: SubscriptionService::new(keys),
            clock,
            engine_registry,
            membership_registry: Arc::new(MembershipRegistryPort(weak.clone())),
            transport_authority: Arc::new(TransportAuthorityPort(weak.clone())),
            source: LogEventSource(weak.clone()),
            retirement: Notify::new(),
            signals,
        }))
    }

    pub(crate) fn now(&self) -> Result<UnixMillis, AsyncErrorKind> {
        self.clock.now().map_err(|_| AsyncErrorKind::Unavailable)
    }

    fn tables(&self) -> MutexGuard<'_, AsyncTables> {
        self.tables
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Removes expired authority and empty transports.
    fn prune(&self, tables: &mut AsyncTables, now: UnixMillis) {
        let expired = tables
            .issued
            .iter()
            .filter(|(_, record)| record.expires_at <= now && record.membership.is_none())
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            remove_issued(tables, &id);
        }
        let stale = tables
            .transports
            .iter()
            .filter(|(key, transport)| {
                !transport.reader_active
                    && transport.credential_expires_at <= now
                    && !tables
                        .issued
                        .values()
                        .any(|record| &record.transport == *key)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(transport) = tables.transports.remove(&key)
                && let Some(credential) = transport.credential
            {
                tables.credentials.remove(&credential);
            }
        }
    }

    pub(crate) fn document_scope(
        &self,
        facts: &HostScopeFacts,
        kind: TransportKind,
    ) -> Result<DocumentAuthorizationScope, AsyncErrorKind> {
        DocumentAuthorizationScope::derive(facts, &transport_policy(kind))
            .map_err(|_| AsyncErrorKind::Unavailable)
    }

    /// Issues one new signed subscription for a validated mount context.
    #[allow(
        clippy::too_many_arguments,
        reason = "issuance keeps every independently trusted authority input explicit"
    )]
    pub(crate) async fn issue(
        self: &Arc<Self>,
        context: &TrustedLiveRequestContext,
        parameters: TrustedMountParameters,
        stream: StreamName,
        kind: TransportKind,
        document_instance: &str,
        origin: VerifiedOrigin,
        baseline: StreamPosition,
    ) -> Result<IssuedView, AsyncErrorKind> {
        let now = self.now()?;
        let expires_at = UnixMillis::new(now.get().saturating_add(SUBSCRIPTION_LIFETIME_MS));
        let component = context.mount().component().clone();
        let contract = context.mount().contract_digest().clone();
        let metadata = self.component_metadata(&component)?;
        let subscription_metadata = metadata
            .subscriptions()
            .iter()
            .find(|candidate| candidate.stream() == &stream)
            .ok_or(AsyncErrorKind::StreamUnknown)?;
        if !subscription_metadata
            .modes()
            .as_slice()
            .contains(&kind.mode())
        {
            return Err(AsyncErrorKind::TransportUnsupported);
        }
        let modes = subscription_metadata.modes().clone();
        let document_scope = self.document_scope(context.host_scope_facts(), kind)?;
        let scope_text = document_scope.to_base64url();
        {
            let mut tables = self.tables();
            self.prune(&mut tables, now);
            let issued_in_scope = tables
                .issued
                .values()
                .filter(|record| record.document_scope == document_scope)
                .count();
            if issued_in_scope >= MAX_ISSUED_PER_SCOPE {
                return Err(AsyncErrorKind::SubscriptionLimit);
            }
        }
        let issued = self
            .service
            .issue(
                context,
                SubscriptionIssueRequest::new(
                    stream.clone(),
                    CapabilityVersion::new(1).map_err(|_| AsyncErrorKind::Unavailable)?,
                    expires_at,
                    fallback_policy()?,
                ),
                now,
            )
            .await
            .map_err(subscription_error)?;
        let authorized = self
            .service
            .connect(
                context,
                issued.descriptor(),
                issued.transport_credential(),
                now,
            )
            .await
            .map_err(subscription_error)?;
        let subscription = SubscriptionId::from_bytes(&random_bytes(16))
            .map_err(|_| AsyncErrorKind::Unavailable)?;
        let id = subscription.to_base64url();
        let envelope_context = self.construct_context(&authorized, subscription.clone(), &id)?;
        let claims = authorized.verified().claims();
        let topics = claims
            .topics()
            .as_slice()
            .iter()
            .map(|topic| topic.as_str().to_owned())
            .collect::<Vec<_>>();
        let binding = authorized.binding().clone();
        let binding_text = binding.to_base64url();
        let descriptor = issued.descriptor().clone();
        let transport_key = TransportKey {
            scope: scope_text,
            kind,
            instance: document_instance.to_owned(),
        };
        let log = Arc::new(Mutex::new(SubscriptionLog::new(
            baseline.epoch().get(),
            now.get(),
        )));
        let mut guard = self.tables();
        let tables = &mut *guard;
        let transports_in_scope = tables
            .transports
            .values()
            .filter(|transport| transport.scope == document_scope)
            .count();
        let credential = if let Some(transport) = tables.transports.get_mut(&transport_key) {
            if transport.credential_expires_at <= now
                && let Some(stale) = transport.credential.take()
            {
                tables.credentials.remove(&stale);
            }
            if kind == TransportKind::Sse && transport.credential.is_none() {
                let minted = mint_credential();
                tables
                    .credentials
                    .insert(minted.clone(), transport_key.clone());
                transport.credential = Some(minted);
            }
            transport.credential_expires_at = transport.credential_expires_at.max(expires_at);
            transport.credential.clone()
        } else {
            if transports_in_scope >= MAX_TRANSPORTS_PER_SCOPE
                || tables.transports.len() >= MAX_TRANSPORTS_TOTAL
            {
                return Err(AsyncErrorKind::TransportLimit);
            }
            let credential = (kind == TransportKind::Sse).then(mint_credential);
            if let Some(credential) = &credential {
                tables
                    .credentials
                    .insert(credential.clone(), transport_key.clone());
            }
            tables.transports.insert(
                transport_key.clone(),
                TransportRecord {
                    kind,
                    scope: document_scope.clone(),
                    origin: origin.clone(),
                    credential: credential.clone(),
                    credential_expires_at: expires_at,
                    handle: DocumentTransportHandle::from_bytes(&random_bytes(16))
                        .map_err(|_| AsyncErrorKind::Unavailable)?,
                    document: Arc::new(tokio::sync::Mutex::new(None)),
                    retained_events: 0,
                    retained_bytes: 0,
                    degraded: false,
                    generation: 0,
                    reader_active: false,
                    outbound: None,
                    wake: Arc::new(Notify::new()),
                    closed: Arc::new(Notify::new()),
                    used_nonces: VecDeque::new(),
                    memberships: BTreeSet::new(),
                    controls_used: 0,
                    coalesced: 0,
                    degraded_lanes: 0,
                },
            );
            credential
        };
        for topic in &topics {
            tables
                .topics
                .entry(topic.clone())
                .or_default()
                .insert(id.clone());
        }
        let view = IssuedView::new(
            &id,
            &binding_text,
            credential,
            kind,
            &document_scope,
            &origin,
            baseline,
            expires_at,
            claims,
            Vec::new(),
            "authoritative_no_tail",
        );
        tables.issued.insert(
            id.clone(),
            IssuedRecord {
                subscription,
                descriptor,
                binding,
                binding_text,
                previous_binding: None,
                authorized: Arc::new(authorized),
                context: Some(envelope_context),
                component,
                contract,
                stream,
                parameters,
                document_scope,
                kind,
                transport: transport_key,
                modes,
                topics,
                log,
                delivery_cursor: Arc::new(AtomicU64::new(1)),
                resume_position: 0,
                expires_at,
                membership: None,
                control_in_flight: false,
                transport_wake: None,
            },
        );
        Ok(view)
    }

    /// Renews one existing subscription from its browser-observed position.
    #[allow(
        clippy::too_many_arguments,
        reason = "renewal keeps every independently trusted authority input explicit"
    )]
    pub(crate) async fn renew(
        self: &Arc<Self>,
        context: &TrustedLiveRequestContext,
        stream: &StreamName,
        kind: TransportKind,
        document_instance: &str,
        prior_id: &str,
        prior_binding: &str,
        position: (u64, u64),
    ) -> Result<IssuedView, AsyncErrorKind> {
        let now = self.now()?;
        let expires_at = UnixMillis::new(now.get().saturating_add(SUBSCRIPTION_LIFETIME_MS));
        let document_scope = self.document_scope(context.host_scope_facts(), kind)?;
        let (descriptor, renewal_credential, log, transport_key, origin) = {
            let mut tables = self.tables();
            let record = tables
                .issued
                .get(prior_id)
                .ok_or(AsyncErrorKind::SubscriptionUnknown)?;
            if record.binding_text != prior_binding
                || record.document_scope != document_scope
                || record.kind != kind
                || record.transport.instance != document_instance
                || &record.stream != stream
                || record.component != *context.mount().component()
            {
                return Err(AsyncErrorKind::SubscriptionUnknown);
            }
            if record.expires_at <= now {
                return Err(AsyncErrorKind::AuthorityExpired);
            }
            {
                let log = lock_log(&record.log);
                if position.0 != log.epoch || position.1 > log.head() {
                    return Err(AsyncErrorKind::PositionInvalid);
                }
            }
            self.prune(&mut tables, now);
            let record = tables
                .issued
                .get(prior_id)
                .ok_or(AsyncErrorKind::SubscriptionUnknown)?;
            let transport = tables
                .transports
                .get(&record.transport)
                .ok_or(AsyncErrorKind::SubscriptionUnknown)?;
            (
                record.descriptor.clone(),
                record
                    .authorized
                    .renewal_credential()
                    .expose_authorization_bearer()
                    .to_vec(),
                Arc::clone(&record.log),
                record.transport.clone(),
                transport.origin.clone(),
            )
        };
        let renewal_credential =
            suprnova_live::async_updates::TransportCredential::from_host_authority_bearer(
                renewal_credential,
            )
            .map_err(|_| AsyncErrorKind::Unavailable)?;
        let renewed = self
            .service
            .renew(context, &descriptor, &renewal_credential, expires_at, now)
            .await
            .map_err(subscription_error)?;
        let authorized = self
            .service
            .connect(
                context,
                renewed.descriptor(),
                renewed.transport_credential(),
                now,
            )
            .await
            .map_err(subscription_error)?;
        let subscription =
            SubscriptionId::parse(prior_id).map_err(|_| AsyncErrorKind::Unavailable)?;
        let envelope_context = self.construct_context(&authorized, subscription, prior_id)?;
        let (replay, proof) = {
            let log = lock_log(&log);
            match log.tail_after(position.0, position.1) {
                Some(tail) if tail.is_empty() => (Vec::new(), "authoritative_no_tail"),
                Some(tail) => (tail, "complete_replay"),
                None => (Vec::new(), "authoritative_no_tail"),
            }
        };
        let mut guard = self.tables();
        let tables = &mut *guard;
        let record = tables
            .issued
            .get_mut(prior_id)
            .ok_or(AsyncErrorKind::SubscriptionUnknown)?;
        let binding = authorized.binding().clone();
        let binding_text = binding.to_base64url();
        record.previous_binding = Some(std::mem::replace(&mut record.binding, binding));
        record.binding_text = binding_text.clone();
        record.descriptor = renewed.descriptor().clone();
        record.authorized = Arc::new(authorized);
        record.context = Some(envelope_context);
        record.expires_at = expires_at;
        record.resume_position = position.1;
        let claims = record.authorized.verified().claims();
        let credential = tables
            .transports
            .get_mut(&transport_key)
            .and_then(|transport| {
                transport.credential_expires_at = transport.credential_expires_at.max(expires_at);
                transport.credential.clone()
            });
        let baseline = StreamPosition::new(
            StreamEpoch::new(position.0),
            StreamSequence::new(position.1),
        );
        let view = IssuedView::new(
            prior_id,
            &binding_text,
            credential,
            kind,
            &document_scope,
            &origin,
            baseline,
            expires_at,
            claims,
            replay,
            proof,
        );
        Ok(view)
    }

    fn construct_context(
        &self,
        authorized: &AuthorizedSubscription,
        subscription: SubscriptionId,
        id: &str,
    ) -> Result<AsyncEnvelopeContext, AsyncErrorKind> {
        let claims = authorized.verified().claims();
        *self
            .constructing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ConstructingClaims {
            subscription: id.to_owned(),
            stream: claims.stream().clone(),
            events: claims.events().clone(),
        });
        let context = AsyncEnvelopeContext::from_authorized(
            authorized,
            subscription,
            self.membership_registry.as_ref(),
        );
        self.constructing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        context.map_err(|_| AsyncErrorKind::Unavailable)
    }

    fn component_metadata(
        &self,
        component: &ComponentName,
    ) -> Result<suprnova_live::metadata::ComponentMetadata, AsyncErrorKind> {
        self.engine_registry
            .resolve(component)
            .map(|descriptor| descriptor.metadata().clone())
            .map_err(|_| AsyncErrorKind::MountUnknown)
    }

    /// Resolves an SSE transport from its document credential.
    pub(crate) fn transport_for_credential(
        &self,
        credential: &str,
        facts: &HostScopeFacts,
    ) -> Result<TransportKey, AsyncErrorKind> {
        let now = self.now()?;
        let tables = self.tables();
        let key = tables
            .credentials
            .get(credential)
            .ok_or(AsyncErrorKind::AuthorityInvalid)?;
        let transport = tables
            .transports
            .get(key)
            .ok_or(AsyncErrorKind::AuthorityInvalid)?;
        let scope = self.document_scope(facts, transport.kind)?;
        if transport.scope != scope {
            return Err(AsyncErrorKind::AuthorityInvalid);
        }
        if transport.credential_expires_at <= now {
            return Err(AsyncErrorKind::AuthorityExpired);
        }
        Ok(key.clone())
    }

    /// Opens the single reader for one SSE transport and returns its outbound channel.
    pub(crate) fn open_sse_reader(
        self: &Arc<Self>,
        key: &TransportKey,
        generation: u64,
    ) -> Result<mpsc::Receiver<Bytes>, AsyncErrorKind> {
        let now = self.now()?;
        let mut tables = self.tables();
        let transport = tables
            .transports
            .get_mut(key)
            .ok_or(AsyncErrorKind::AuthorityInvalid)?;
        if transport.kind != TransportKind::Sse {
            return Err(AsyncErrorKind::TransportMismatch);
        }
        if transport.credential_expires_at <= now {
            return Err(AsyncErrorKind::AuthorityExpired);
        }
        if transport.reader_active {
            return Err(AsyncErrorKind::TransportReaderExists);
        }
        let (sender, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
        let document = new_document(
            transport.origin.clone(),
            DocumentTransportKind::ServerSentEvents,
            transport.handle.clone(),
            transport.scope.clone(),
        )?;
        transport.document = Arc::new(tokio::sync::Mutex::new(Some(document)));
        transport.retained_events = 0;
        transport.retained_bytes = 0;
        transport.degraded = false;
        transport.generation = generation;
        transport.reader_active = true;
        transport.outbound = Some(sender.clone());
        transport.used_nonces.clear();
        transport.controls_used = 0;
        let wake = Arc::clone(&transport.wake);
        let closed = Arc::clone(&transport.closed);
        drop(tables);
        if sender
            .try_send(Bytes::from_static(SseEncoder::heartbeat_comment()))
            .is_err()
        {
            return Err(AsyncErrorKind::Unavailable);
        }
        spawn_delivery_loop(
            Arc::clone(self),
            key.clone(),
            generation,
            wake,
            closed,
            sender,
        );
        Ok(receiver)
    }

    /// Creates one WebSocket transport for an upgraded same-origin socket.
    pub(crate) fn open_socket(
        self: &Arc<Self>,
        facts: &HostScopeFacts,
        origin: VerifiedOrigin,
    ) -> Result<(TransportKey, mpsc::Receiver<Bytes>), AsyncErrorKind> {
        let now = self.now()?;
        let scope = self.document_scope(facts, TransportKind::WebSocket)?;
        let key = TransportKey {
            scope: scope.to_base64url(),
            kind: TransportKind::WebSocket,
            instance: URL_SAFE_NO_PAD.encode(random_bytes(16)),
        };
        let mut tables = self.tables();
        self.prune(&mut tables, now);
        let transports_in_scope = tables
            .transports
            .values()
            .filter(|transport| transport.scope == scope)
            .count();
        if transports_in_scope >= MAX_TRANSPORTS_PER_SCOPE
            || tables.transports.len() >= MAX_TRANSPORTS_TOTAL
        {
            return Err(AsyncErrorKind::TransportLimit);
        }
        let handle = DocumentTransportHandle::from_bytes(&random_bytes(16))
            .map_err(|_| AsyncErrorKind::Unavailable)?;
        let document = new_document(
            origin.clone(),
            DocumentTransportKind::WebSocket,
            handle.clone(),
            scope.clone(),
        )?;
        let (sender, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
        let wake = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        tables.transports.insert(
            key.clone(),
            TransportRecord {
                kind: TransportKind::WebSocket,
                scope,
                origin,
                credential: None,
                credential_expires_at: UnixMillis::new(
                    now.get().saturating_add(SUBSCRIPTION_LIFETIME_MS),
                ),
                handle,
                document: Arc::new(tokio::sync::Mutex::new(Some(document))),
                retained_events: 0,
                retained_bytes: 0,
                degraded: false,
                generation: 0,
                reader_active: true,
                outbound: Some(sender.clone()),
                wake: Arc::clone(&wake),
                closed: Arc::clone(&closed),
                used_nonces: VecDeque::new(),
                memberships: BTreeSet::new(),
                controls_used: 0,
                coalesced: 0,
                degraded_lanes: 0,
            },
        );
        drop(tables);
        spawn_delivery_loop(Arc::clone(self), key.clone(), 0, wake, closed, sender);
        Ok((key, receiver))
    }

    /// Validates and remembers one SSE control nonce for the current reader generation.
    pub(crate) fn admit_sse_control(
        &self,
        key: &TransportKey,
        generation: u64,
        nonce: &str,
    ) -> Result<(), AsyncErrorKind> {
        let mut tables = self.tables();
        let transport = tables
            .transports
            .get_mut(key)
            .ok_or(AsyncErrorKind::AuthorityInvalid)?;
        if !transport.reader_active {
            return Err(AsyncErrorKind::TransportClosed);
        }
        if transport.generation != generation {
            return Err(AsyncErrorKind::GenerationStale);
        }
        if !transport.remember_nonce(nonce) {
            return Err(AsyncErrorKind::ControlReplayed);
        }
        Ok(())
    }

    /// Binds a WebSocket transport to its first browser generation and counts controls.
    pub(crate) fn admit_socket_control(
        &self,
        key: &TransportKey,
        generation: u64,
    ) -> Result<(), AsyncErrorKind> {
        let mut tables = self.tables();
        let transport = tables
            .transports
            .get_mut(key)
            .ok_or(AsyncErrorKind::TransportClosed)?;
        if transport.controls_used >= MAX_SOCKET_CONTROLS {
            return Err(AsyncErrorKind::ControlCapacityExceeded);
        }
        transport.controls_used += 1;
        if transport.generation == 0 {
            transport.generation = generation;
        } else if transport.generation != generation {
            return Err(AsyncErrorKind::GenerationStale);
        }
        Ok(())
    }

    /// Counts one WebSocket unsubscribe control.
    pub(crate) fn count_socket_control(&self, key: &TransportKey) -> Result<(), AsyncErrorKind> {
        let mut tables = self.tables();
        let transport = tables
            .transports
            .get_mut(key)
            .ok_or(AsyncErrorKind::TransportClosed)?;
        if transport.controls_used >= MAX_SOCKET_CONTROLS {
            return Err(AsyncErrorKind::ControlCapacityExceeded);
        }
        transport.controls_used += 1;
        Ok(())
    }

    /// Adds one exact logical membership to a transport through the engine pipeline.
    pub(crate) async fn add_membership(
        self: &Arc<Self>,
        key: &TransportKey,
        subscription_id: &str,
        expected: Option<(&str, &str)>,
        socket_request: Option<WebSocketMembershipRequest>,
    ) -> Result<Option<WebSocketMembershipAcknowledgment>, AsyncErrorKind> {
        let now = self.now()?;
        let socket = socket_request.is_some();
        let (rotated, origin, handle, slot, authorized, subscription, document_scope, modes) = {
            let mut tables = self.tables();
            let transport = tables
                .transports
                .get(key)
                .ok_or(AsyncErrorKind::TransportClosed)?;
            if !transport.reader_active {
                return Err(AsyncErrorKind::TransportClosed);
            }
            let origin = transport.origin.clone();
            let handle = transport.handle.clone();
            let slot = Arc::clone(&transport.document);
            let transport_kind = transport.kind;
            let record = tables
                .issued
                .get_mut(subscription_id)
                .ok_or(AsyncErrorKind::MembershipInvalid)?;
            if record.kind != transport_kind {
                return Err(AsyncErrorKind::TransportMismatch);
            }
            let same_transport = if socket {
                record.transport.scope == key.scope
            } else {
                record.transport == *key
            };
            if !same_transport || !expected_matches(record, expected) {
                return Err(AsyncErrorKind::MembershipInvalid);
            }
            if record.expires_at <= now {
                return Err(AsyncErrorKind::AuthorityExpired);
            }
            if record.control_in_flight {
                return Err(AsyncErrorKind::ControlInFlight);
            }
            let rotated = match &record.membership {
                Some(active) if active.binding() == &record.binding => {
                    return Err(AsyncErrorKind::MembershipDuplicate);
                }
                Some(_) => true,
                None => false,
            };
            record.control_in_flight = true;
            (
                rotated,
                origin,
                handle,
                slot,
                Arc::clone(&record.authorized),
                record.subscription.clone(),
                record.document_scope.clone(),
                record.modes.clone(),
            )
        };
        let lease = MembershipLease::held(self, subscription_id);
        if rotated {
            self.remove_committed_membership(key, subscription_id, socket)
                .await?;
        }
        let authorization = AuthorizedTransportSubscription::new(
            &authorized,
            subscription,
            self.membership_registry.as_ref(),
            origin.clone(),
            document_scope,
            modes,
            Arc::clone(&self.transport_authority) as Arc<dyn AsyncTransportAuthorityPort>,
            now,
        )
        .map_err(transport_error)?;
        let stored = authorization.clone();
        let acknowledgment = match socket_request {
            None => {
                let pending = {
                    let mut guard = slot.lock().await;
                    let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                    SseMembershipControl::prepare_subscribe(
                        document.transport(),
                        &handle,
                        &origin,
                        authorization,
                    )
                    .map_err(transport_error)?
                };
                let authorized = pending.authorize().await.map_err(transport_error)?;
                let establishing = {
                    let mut guard = slot.lock().await;
                    let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                    document
                        .transport()
                        .prepare_establish(authorized)
                        .map_err(transport_error)?
                };
                let ready = establishing
                    .establish(&self.source)
                    .await
                    .map_err(transport_error)?;
                let mut guard = slot.lock().await;
                let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                document.commit_add(ready).map_err(transport_error)?;
                None
            }
            Some(request) => {
                let pending = {
                    let mut guard = slot.lock().await;
                    let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                    WebSocketMembershipControl::prepare_authenticated_subscribe(
                        document.transport(),
                        request,
                        authorization,
                    )
                    .map_err(transport_error)?
                };
                let authorized = pending.authorize().await.map_err(transport_error)?;
                let establishing = {
                    let mut guard = slot.lock().await;
                    let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                    authorized
                        .prepare_establish(document.transport())
                        .map_err(transport_error)?
                };
                let ready = establishing
                    .establish(&self.source)
                    .await
                    .map_err(transport_error)?;
                let mut guard = slot.lock().await;
                let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
                let receipt = WebSocketMembershipControl::commit_authenticated_bounded_subscribe(
                    document, ready,
                )
                .map_err(transport_error)?;
                Some(WebSocketMembershipControl::acknowledge_committed(receipt))
            }
        };
        self.finish_add(key, &slot, subscription_id, stored).await;
        drop(lease);
        Ok(acknowledgment)
    }

    /// Records one committed membership and re-baselines a resumed lane.
    async fn finish_add(
        &self,
        key: &TransportKey,
        slot: &DocumentSlot,
        subscription_id: &str,
        authorization: AuthorizedTransportSubscription,
    ) {
        let (wake, resume, baseline, epoch) = {
            let mut tables = self.tables();
            let Some(transport) = tables.transports.get_mut(key) else {
                return;
            };
            transport.memberships.insert(subscription_id.to_owned());
            let wake = Arc::clone(&transport.wake);
            let Some(record) = tables.issued.get_mut(subscription_id) else {
                return;
            };
            record.transport = key.clone();
            record.transport_wake = Some(Arc::clone(&wake));
            let baseline = authorization.baseline().sequence().get();
            let resume = record.resume_position;
            let epoch = lock_log(&record.log).epoch;
            record.membership = Some(authorization.clone());
            (wake, resume, baseline, epoch)
        };
        if resume > baseline {
            let mut guard = slot.lock().await;
            if let Some(document) = guard.as_mut() {
                let _ = document.recover_from_authoritative_refresh(
                    &authorization,
                    self.membership_registry.as_ref(),
                    &FixedContinuity(StreamPosition::new(
                        StreamEpoch::new(epoch),
                        StreamSequence::new(resume),
                    )),
                );
            }
        }
        wake.notify_one();
    }

    /// Removes one exact logical membership through the engine pipeline.
    pub(crate) async fn remove_membership(
        self: &Arc<Self>,
        key: &TransportKey,
        subscription_id: &str,
        expected: Option<(&str, &str)>,
        socket: bool,
    ) -> Result<(), AsyncErrorKind> {
        {
            let mut tables = self.tables();
            let record = tables
                .issued
                .get_mut(subscription_id)
                .ok_or(AsyncErrorKind::MembershipUnknown)?;
            if !expected_matches(record, expected) {
                return Err(AsyncErrorKind::MembershipInvalid);
            }
            if record.control_in_flight {
                return Err(AsyncErrorKind::ControlInFlight);
            }
            if record.membership.is_none() || record.transport != *key {
                return Err(AsyncErrorKind::MembershipUnknown);
            }
            record.control_in_flight = true;
        }
        let lease = MembershipLease::held(self, subscription_id);
        let result = self
            .remove_committed_membership(key, subscription_id, socket)
            .await;
        drop(lease);
        result
    }

    async fn remove_committed_membership(
        self: &Arc<Self>,
        key: &TransportKey,
        subscription_id: &str,
        socket: bool,
    ) -> Result<(), AsyncErrorKind> {
        let (authorization, handle, origin, slot) = {
            let tables = self.tables();
            let record = tables
                .issued
                .get(subscription_id)
                .ok_or(AsyncErrorKind::MembershipUnknown)?;
            let authorization = record
                .membership
                .clone()
                .ok_or(AsyncErrorKind::MembershipUnknown)?;
            let transport = tables
                .transports
                .get(key)
                .ok_or(AsyncErrorKind::TransportClosed)?;
            (
                authorization,
                transport.handle.clone(),
                transport.origin.clone(),
                Arc::clone(&transport.document),
            )
        };
        let pending = {
            let mut guard = slot.lock().await;
            let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
            if socket {
                WebSocketMembershipControl::prepare_unsubscribe(
                    document.transport(),
                    &WebSocketControlRecord::Unsubscribe(authorization.subscription().clone()),
                    &authorization,
                )
                .map_err(transport_error)?
            } else {
                SseMembershipControl::prepare_unsubscribe(
                    document.transport(),
                    &handle,
                    &origin,
                    &authorization,
                )
                .map_err(transport_error)?
            }
        };
        let ready = pending.authorize().await.map_err(transport_error)?;
        {
            let mut guard = slot.lock().await;
            let document = guard.as_mut().ok_or(AsyncErrorKind::TransportClosed)?;
            document.commit_remove(ready).map_err(transport_error)?;
        }
        let mut tables = self.tables();
        if let Some(transport) = tables.transports.get_mut(key) {
            transport.memberships.remove(subscription_id);
        }
        if let Some(record) = tables.issued.get_mut(subscription_id) {
            record.membership = None;
            record.transport_wake = None;
        }
        Ok(())
    }

    /// Publishes one typed payload to every subscription of `topic`.
    pub(crate) fn publish(
        &self,
        topic: &str,
        spec: &StreamPayloadSpec,
    ) -> Result<(), PublishError> {
        TopicName::parse(topic).map_err(|_| PublishError::InvalidTopic)?;
        let now = self.now().map_err(|_| PublishError::InvalidPayload)?;
        let mut tables = self.tables();
        self.prune(&mut tables, now);
        let Some(ids) = tables.topics.get(topic).cloned() else {
            return Ok(());
        };
        let mut accepted = 0_usize;
        let mut rejected = 0_usize;
        for id in ids {
            let Some(record) = tables.issued.get(&id) else {
                continue;
            };
            let Some(context) = record.context() else {
                continue;
            };
            let payload = match spec {
                StreamPayloadSpec::Refresh => AsyncPayload::Refresh(RegisteredRefresh),
                StreamPayloadSpec::BrowserEvent {
                    name,
                    version,
                    target,
                    payload,
                } => {
                    let Ok(name) = BrowserOperationName::parse(name) else {
                        rejected += 1;
                        continue;
                    };
                    let Some(target) = engine_target(target) else {
                        rejected += 1;
                        continue;
                    };
                    match RegisteredBrowserEvent::new(
                        context,
                        name,
                        *version,
                        target,
                        payload.clone(),
                    ) {
                        Ok(event) => AsyncPayload::BrowserEvent(event),
                        Err(_) => {
                            rejected += 1;
                            continue;
                        }
                    }
                }
            };
            if append_payload(record, payload, now).is_ok() {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
        if accepted == 0 && rejected > 0 {
            return Err(PublishError::InvalidPayload);
        }
        Ok(())
    }

    /// Appends heartbeat continuity to idle memberships of one transport.
    fn heartbeat(&self, key: &TransportKey) {
        let Ok(now) = self.now() else {
            return;
        };
        let tables = self.tables();
        let Some(transport) = tables.transports.get(key) else {
            return;
        };
        for id in &transport.memberships {
            let Some(record) = tables.issued.get(id) else {
                continue;
            };
            let idle = {
                let log = lock_log(&record.log);
                now.get().saturating_sub(log.last_append_ms)
                    >= u64::try_from(HEARTBEAT_INTERVAL.as_millis()).unwrap_or(u64::MAX)
            };
            if idle {
                let _ = append_payload(record, AsyncPayload::Heartbeat(Heartbeat), now);
            }
        }
    }

    /// Retires one transport: closes the document and releases every membership.
    pub(crate) async fn retire_transport(self: &Arc<Self>, key: &TransportKey, generation: u64) {
        let slot = {
            let mut tables = self.tables();
            let Some(transport) = tables.transports.get_mut(key) else {
                return;
            };
            if transport.generation != generation && transport.kind == TransportKind::Sse {
                return;
            }
            transport.reader_active = false;
            transport.outbound = None;
            let slot = Arc::clone(&transport.document);
            let memberships = std::mem::take(&mut transport.memberships);
            let forget = transport.kind == TransportKind::WebSocket;
            transport.closed.notify_waiters();
            for id in memberships {
                if let Some(record) = tables.issued.get_mut(&id) {
                    record.membership = None;
                    record.transport_wake = None;
                }
            }
            if forget {
                tables.transports.remove(key);
            }
            slot
        };
        let document = slot.lock().await.take();
        if let Some(mut document) = document {
            let _ = document.close().await;
        }
        self.retirement.notify_waiters();
    }

    /// Waits until the transport bound to `credential` has no active reader.
    pub(crate) async fn await_retirement(&self, credential: &str) {
        loop {
            let notified = self.retirement.notified();
            let active = {
                let tables = self.tables();
                tables
                    .credentials
                    .get(credential)
                    .and_then(|key| tables.transports.get(key))
                    .is_some_and(|transport| transport.reader_active)
            };
            if !active {
                return;
            }
            notified.await;
        }
    }

    /// Returns bounded low-cardinality reports for every transport.
    pub(crate) fn reports(&self) -> Vec<TransportReportData> {
        let tables = self.tables();
        tables
            .transports
            .values()
            .map(|transport| TransportReportData {
                kind: transport.kind.as_str(),
                credential: transport.credential.clone(),
                memberships: transport.memberships.len(),
                retained_events: transport.retained_events,
                retained_bytes: transport.retained_bytes,
                degraded: transport.degraded,
                reader_active: transport.reader_active,
                coalesced: transport.coalesced,
                degraded_lanes: transport.degraded_lanes,
            })
            .collect()
    }
}

/// Bounded observation of one transport for tests.
pub(crate) struct TransportReportData {
    pub(crate) kind: &'static str,
    pub(crate) credential: Option<String>,
    pub(crate) memberships: usize,
    pub(crate) retained_events: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) degraded: bool,
    pub(crate) reader_active: bool,
    pub(crate) coalesced: u64,
    pub(crate) degraded_lanes: u64,
}

fn expected_matches(record: &IssuedRecord, expected: Option<(&str, &str)>) -> bool {
    expected.is_none_or(|(binding, stream)| {
        record.binding_text == binding && record.stream.as_str() == stream
    })
}

struct MembershipLease {
    state: Arc<AsyncState>,
    subscription: String,
}

impl MembershipLease {
    /// Wraps an already-raised in-flight flag so any exit clears it exactly once.
    fn held(state: &Arc<AsyncState>, subscription: &str) -> Self {
        Self {
            state: Arc::clone(state),
            subscription: subscription.to_owned(),
        }
    }
}

impl Drop for MembershipLease {
    fn drop(&mut self) {
        if let Some(record) = self.state.tables().issued.get_mut(&self.subscription) {
            record.control_in_flight = false;
        }
    }
}

fn append_payload(record: &IssuedRecord, payload: AsyncPayload, now: UnixMillis) -> Result<(), ()> {
    let context = record.context().ok_or(())?;
    let mut log = lock_log(&record.log);
    let position = log.next_position();
    let envelope = AsyncEnvelope::new(context, position, payload).map_err(|_| ())?;
    let encoded = encode_async_envelope(&envelope, &AsyncCodecLimits::v1()).map_err(|_| ())?;
    log.append(envelope, Bytes::from(encoded), now.get());
    drop(log);
    if let Some(wake) = &record.transport_wake {
        wake.notify_one();
    }
    Ok(())
}

fn remove_issued(tables: &mut AsyncTables, id: &str) {
    if let Some(record) = tables.issued.remove(id) {
        for topic in record.topics {
            if let Some(ids) = tables.topics.get_mut(&topic) {
                ids.remove(id);
                if ids.is_empty() {
                    tables.topics.remove(&topic);
                }
            }
        }
    }
}

fn lock_log(log: &Arc<Mutex<SubscriptionLog>>) -> MutexGuard<'_, SubscriptionLog> {
    log.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn new_document(
    origin: VerifiedOrigin,
    kind: DocumentTransportKind,
    handle: DocumentTransportHandle,
    scope: DocumentAuthorizationScope,
) -> Result<BoundedDocumentTransportSession, AsyncErrorKind> {
    let limits = DocumentTransportLimits::new(MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS)
        .map_err(|_| AsyncErrorKind::Unavailable)?;
    let transport = DocumentTransportSession::new(origin, kind, handle, limits, scope);
    BoundedDocumentTransportSession::new(
        transport,
        ResourceBounds::new(MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_BUFFER_BYTES)
            .map_err(|_| AsyncErrorKind::Unavailable)?,
        PermitPool::new(1).map_err(|_| AsyncErrorKind::Unavailable)?,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(MAX_ASYNC_PAYLOAD_BYTES)
                .ok_or(AsyncErrorKind::Unavailable)?,
            max_replay_events: NonZeroUsize::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES)
                .ok_or(AsyncErrorKind::Unavailable)?,
            max_fanout: NonZeroUsize::new(usize::from(MAX_EVENT_FANOUT))
                .ok_or(AsyncErrorKind::Unavailable)?,
        },
    )
    .map_err(|_| AsyncErrorKind::Unavailable)
}

fn transport_policy(kind: TransportKind) -> ContentDigest {
    let mut digest = Sha256::new();
    digest.update(TRANSPORT_POLICY_PURPOSE);
    digest.update(kind.as_str().as_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    ContentDigest::from_bytes(&bytes).expect("fixed digest width")
}

fn target_scope(handle: &DocumentTransportHandle, target: &EventTarget) -> ContentDigest {
    let mut digest = Sha256::new();
    digest.update(TARGET_SCOPE_PURPOSE);
    digest.update(handle.to_base64url().as_bytes());
    digest.update([0]);
    digest.update(format!("{target:?}").as_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    ContentDigest::from_bytes(&bytes).expect("fixed digest width")
}

fn engine_target(target: &LiveEventTarget) -> Option<EventTarget> {
    Some(match target {
        LiveEventTarget::Island => EventTarget::SelfIsland,
        LiveEventTarget::Parent => EventTarget::Parent,
        LiveEventTarget::Child => EventTarget::Child,
        LiveEventTarget::Document => EventTarget::Document,
        LiveEventTarget::NamedIsland(slot) => {
            EventTarget::NamedIsland(IslandSlot::parse(slot).ok()?)
        }
        LiveEventTarget::Browser(listener) => {
            EventTarget::Browser(BrowserOperationName::parse(listener).ok()?)
        }
    })
}

fn fallback_policy() -> Result<PollFallbackPolicy, AsyncErrorKind> {
    PollFallbackPolicy::new(
        POLL_INTERVAL_MS,
        POLL_JITTER_BASIS_POINTS,
        PollInitialBehavior::AfterInterval,
        PollVisibilityPolicy::PauseWhenHidden,
    )
    .map_err(|_| AsyncErrorKind::Unavailable)
}

fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count);
    while bytes.len() < count {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(count);
    bytes
}

fn mint_credential() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes(32))
}

fn subscription_error(error: SubscriptionError) -> AsyncErrorKind {
    match error.kind() {
        SubscriptionErrorKind::AuthorizationDenied => AsyncErrorKind::AuthorizationDenied,
        SubscriptionErrorKind::UnregisteredSubscription => AsyncErrorKind::StreamUnknown,
        SubscriptionErrorKind::DescriptorExpired | SubscriptionErrorKind::ContextExpired => {
            AsyncErrorKind::AuthorityExpired
        }
        SubscriptionErrorKind::ScopeMismatch | SubscriptionErrorKind::InvalidCredential => {
            AsyncErrorKind::AuthorityInvalid
        }
        _ => AsyncErrorKind::Unavailable,
    }
}

fn transport_error(error: AsyncTransportError) -> AsyncErrorKind {
    match error.kind() {
        AsyncTransportErrorKind::MembershipLimit => AsyncErrorKind::MembershipLimit,
        AsyncTransportErrorKind::DuplicateMembership => AsyncErrorKind::MembershipDuplicate,
        AsyncTransportErrorKind::UnknownMembership => AsyncErrorKind::MembershipUnknown,
        AsyncTransportErrorKind::TransportMismatch => AsyncErrorKind::TransportMismatch,
        AsyncTransportErrorKind::Closed | AsyncTransportErrorKind::StaleControl => {
            AsyncErrorKind::TransportClosed
        }
        AsyncTransportErrorKind::SourceFailed | AsyncTransportErrorKind::BaselineMismatch => {
            AsyncErrorKind::Unavailable
        }
        _ => AsyncErrorKind::MembershipInvalid,
    }
}

/// Projects one issued subscription onto the browser adapter contract.
pub(crate) struct IssuedView {
    pub(crate) value: serde_json::Value,
}

impl IssuedView {
    #[allow(
        clippy::too_many_arguments,
        reason = "the browser contract projection names every independently trusted field"
    )]
    fn new(
        id: &str,
        binding: &str,
        credential: Option<String>,
        kind: TransportKind,
        document_scope: &DocumentAuthorizationScope,
        origin: &VerifiedOrigin,
        baseline: StreamPosition,
        expires_at: UnixMillis,
        claims: &suprnova_live::async_updates::SubscriptionClaims,
        replay: Vec<Bytes>,
        proof: &str,
    ) -> Self {
        use serde_json::json;
        let authorization = match credential {
            Some(credential) => json!({ "kind": "bearer", "credential": credential }),
            None => json!({ "kind": "session_cookie" }),
        };
        let events = claims
            .events()
            .as_slice()
            .iter()
            .map(|event| {
                json!({
                    "cycle": match event.cycle() {
                        suprnova_live::async_updates::EventCyclePolicy::ForbidRepeatedIsland => {
                            json!({ "kind": "forbid_repeated_island" })
                        }
                        suprnova_live::async_updates::EventCyclePolicy::MaximumHops(hops) => {
                            json!({ "kind": "maximum_hops", "maximumHops": hops.get() })
                        }
                    },
                    "maximumFanout": event.maximum_fanout().get(),
                    "name": event.name().as_str(),
                    "order": "per_source_sequence",
                    "payloadContract": event.payload_contract().as_str(),
                    "schema": schema_name(event.schema()),
                    "source": "stream",
                    "targets": event
                        .targets()
                        .as_slice()
                        .iter()
                        .map(target_name)
                        .collect::<Vec<_>>(),
                    "version": event.version(),
                })
            })
            .collect::<Vec<_>>();
        let (reconnect_kind, attempts) = match claims.reconnect() {
            suprnova_live::async_updates::ReconnectPolicy::RefreshOnReconnect => {
                ("refresh_on_reconnect", DEFAULT_RECONNECT_ATTEMPTS)
            }
            suprnova_live::async_updates::ReconnectPolicy::ResumeOrRefresh { maximum_attempts } => {
                ("resume_or_refresh", maximum_attempts.get())
            }
        };
        let fallback = claims.fallback_poll();
        let replay = replay
            .iter()
            .map(|encoded| String::from_utf8_lossy(encoded).into_owned())
            .collect::<Vec<_>>();
        Self {
            value: json!({
                "proof": proof,
                "replay": replay,
                "subscription": {
                    "authorization": authorization,
                    "baseline": {
                        "epoch": baseline.epoch().get().to_string(),
                        "sequence": baseline.sequence().get().to_string(),
                    },
                    "descriptor_binding": binding,
                    "document": {
                        "authorization_scope": document_scope.to_base64url(),
                        "origin": origin.to_string(),
                        "transport": kind.as_str(),
                    },
                    "events": events,
                    "expires_at": expires_at.get(),
                    "fallback_poll": {
                        "initial": match fallback.initial() {
                            PollInitialBehavior::Immediate => "immediate",
                            PollInitialBehavior::AfterInterval => "wait",
                        },
                        "interval_ms": fallback.interval_ms(),
                        "jitter_ratio": f64::from(fallback.jitter_basis_points()) / 10_000.0,
                        "visibility": match fallback.visibility() {
                            PollVisibilityPolicy::PauseWhenHidden => "visible",
                            PollVisibilityPolicy::ContinueWhenHidden => "always",
                        },
                    },
                    "heartbeat_timeout_ms": HEARTBEAT_TIMEOUT_MS,
                    "presentation_signals": [],
                    "reconnect": {
                        "kind": reconnect_kind,
                        "maximum_attempts": attempts,
                        "maximum_delay_ms": RECONNECT_MAXIMUM_DELAY_MS,
                        "minimum_delay_ms": RECONNECT_MINIMUM_DELAY_MS,
                    },
                    "stream": claims.stream().as_str(),
                    "subscription_id": id,
                },
            }),
        }
    }
}

fn schema_name(schema: suprnova_live::async_updates::BrowserPayloadSchema) -> &'static str {
    use suprnova_live::async_updates::BrowserPayloadSchema;
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

struct MembershipRegistryPort(Weak<AsyncState>);

impl AsyncMembershipRegistryPort for MembershipRegistryPort {
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    ) {
        let Some(state) = self.0.upgrade() else {
            return;
        };
        let id = request.subscription().to_base64url();
        {
            let constructing = state
                .constructing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(claims) = constructing.as_ref()
                && claims.subscription == id
                && request.envelope().is_none()
                && request.binding().is_none()
            {
                validation.accept_current(&claims.stream, &claims.events, &state.signals);
                return;
            }
        }
        let tables = state.tables();
        let Some(record) = tables.issued.get(&id) else {
            return;
        };
        let claims = record.authorized.verified().claims();
        if let Some(envelope) = request.envelope() {
            let resolved = match envelope.payload() {
                AsyncPayload::BrowserEvent(event) => {
                    let Some(transport) = tables.transports.get(&record.transport) else {
                        return;
                    };
                    let recipients = match event.target() {
                        EventTarget::SelfIsland
                        | EventTarget::Parent
                        | EventTarget::NamedIsland(_) => 1,
                        EventTarget::Child | EventTarget::Document | EventTarget::Browser(_) => {
                            u16::try_from(transport.memberships.len().max(1))
                                .unwrap_or(u16::MAX)
                                .min(event.maximum_fanout().get())
                        }
                    };
                    NonZeroU16::new(recipients).map(|recipients| {
                        ResolvedEventFanout::from_host(
                            recipients,
                            target_scope(&transport.handle, event.target()),
                        )
                    })
                }
                _ => None,
            };
            validation.accept_delivery_current(
                claims.stream(),
                claims.events(),
                &state.signals,
                claims.authorization_memo(),
                &record.document_scope,
                resolved,
            );
        } else if request.binding().is_some() {
            validation.accept_scope_current(
                claims.stream(),
                claims.events(),
                &state.signals,
                claims.authorization_memo(),
                &record.document_scope,
            );
        } else {
            validation.accept_current(claims.stream(), claims.events(), &state.signals);
        }
    }
}

struct TransportAuthorityPort(Weak<AsyncState>);

impl AsyncTransportAuthorityPort for TransportAuthorityPort {
    fn now(&self) -> UnixMillis {
        self.0
            .upgrade()
            .and_then(|state| state.clock.now().ok())
            .unwrap_or(UnixMillis::new(u64::MAX))
    }

    fn validate_current<'a>(
        &'a self,
        request: AsyncTransportAuthorityRequest<'a>,
        validation: &'a mut AsyncTransportAuthorityValidation,
    ) -> AsyncTransportFuture<'a, ()> {
        Box::pin(async move {
            let Some(state) = self.0.upgrade() else {
                return;
            };
            let Ok(now) = state.clock.now() else {
                return;
            };
            let tables = state.tables();
            let Some(record) = tables.issued.get(&request.subscription().to_base64url()) else {
                return;
            };
            if !record.binding_matches(request.binding())
                || record.expires_at <= now
                || record.document_scope != *request.document_scope()
                || record.kind.document_kind() != request.document_kind()
            {
                return;
            }
            let Some(transport) = tables
                .transports
                .values()
                .find(|transport| transport.handle == *request.document_handle())
            else {
                return;
            };
            if transport.origin != *request.document_origin()
                || transport.scope != record.document_scope
            {
                return;
            }
            let Ok(descriptor) = state.engine_registry.resolve(&record.component) else {
                return;
            };
            if descriptor.contract_digest() != &record.contract {
                return;
            }
            let Ok(registration) = CurrentSubscriptionRegistration::from_registered(
                descriptor.metadata(),
                &record.stream,
                &record.parameters,
            ) else {
                return;
            };
            let claims = request.descriptor().claims();
            validation.accept_current(
                &record.document_scope,
                claims.authorization_memo(),
                registration.stream(),
                registration.topics(),
                registration.events(),
                &record.modes,
            );
        })
    }
}

struct LogEventSource(Weak<AsyncState>);

impl AsyncEventSource for LogEventSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>>
    {
        Box::pin(async move {
            let state = self
                .0
                .upgrade()
                .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed))?;
            let tables = state.tables();
            let record = tables
                .issued
                .get(&request.subscription().to_base64url())
                .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed))?;
            let start = request
                .baseline()
                .sequence()
                .get()
                .max(record.resume_position);
            record
                .delivery_cursor
                .store(start.saturating_add(1), Ordering::Release);
            Ok(Box::pin(LogSession {
                log: Arc::clone(&record.log),
                baseline: request.baseline(),
                cursor: start.saturating_add(1),
                delivery_cursor: Arc::clone(&record.delivery_cursor),
                closed: false,
            }) as Pin<Box<dyn AsyncEventSession>>)
        })
    }
}

struct FixedContinuity(StreamPosition);

impl AsyncContinuityAuthorityPort for FixedContinuity {
    fn authoritative_refresh(
        &self,
        _request: AsyncContinuityRequest<'_>,
    ) -> Option<StreamPosition> {
        Some(self.0)
    }
}

#[derive(Default)]
struct FrameCollector {
    frames: Vec<Bytes>,
    sse: bool,
}

impl AsyncEnvelopeDispatchPort for FrameCollector {
    fn dispatch(&mut self, delivery: ResolvedAsyncDelivery<'_>) -> Result<(), AsyncDispatchError> {
        let frame = if self.sse {
            SseEncoder::encode_envelope(delivery.envelope())
                .map(|event| Bytes::copy_from_slice(event.as_bytes()))
                .map_err(|_| AsyncDispatchError::rejected())?
        } else {
            WebSocketCodec::v1()
                .encode_envelope(delivery.envelope())
                .map(Bytes::from)
                .map_err(|_| AsyncDispatchError::rejected())?
        };
        self.frames.push(frame);
        Ok(())
    }
}

fn spawn_delivery_loop(
    state: Arc<AsyncState>,
    key: TransportKey,
    generation: u64,
    wake: Arc<Notify>,
    closed: Arc<Notify>,
    sender: mpsc::Sender<Bytes>,
) {
    tokio::spawn(async move {
        let sse = key.kind == TransportKind::Sse;
        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut trailer = tokio::time::interval(SSE_DELIVERY_TRAILER_DELAY);
        trailer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut trailer_pending = false;
        loop {
            let (frames, mut retire) = drain_transport(&state, &key, generation, sse).await;
            if sse && !frames.is_empty() {
                trailer.reset();
                trailer_pending = true;
            }
            for frame in frames {
                if sender.send(frame).await.is_err() {
                    retire = true;
                    break;
                }
            }
            if retire {
                state.retire_transport(&key, generation).await;
                break;
            }
            tokio::select! {
                () = wake.notified() => {}
                () = closed.notified() => break,
                () = sender.closed() => {
                    state.retire_transport(&key, generation).await;
                    break;
                }
                _ = heartbeat.tick() => {
                    state.heartbeat(&key);
                }
                _ = trailer.tick(), if trailer_pending => {
                    trailer_pending = false;
                    if sender
                        .send(Bytes::from_static(SseEncoder::heartbeat_comment()))
                        .await
                        .is_err()
                    {
                        state.retire_transport(&key, generation).await;
                        break;
                    }
                }
            }
        }
    });
}

/// Admits and dispatches one bounded batch of deliveries for one document.
async fn drain_transport(
    state: &Arc<AsyncState>,
    key: &TransportKey,
    generation: u64,
    sse: bool,
) -> (Vec<Bytes>, bool) {
    let slot = {
        let tables = state.tables();
        let Some(transport) = tables.transports.get(key) else {
            return (Vec::new(), true);
        };
        if transport.generation != generation && sse {
            return (Vec::new(), true);
        }
        Arc::clone(&transport.document)
    };
    let mut guard = slot.lock().await;
    let Some(document) = guard.as_mut() else {
        return (Vec::new(), true);
    };
    let mut frames = Vec::new();
    let mut retire = false;
    let mut coalesced = 0_u64;
    let mut degraded = 0_u64;
    let mut reconcile = false;
    let registry = state.membership_registry.as_ref();
    for _ in 0..DELIVERY_BATCH {
        match document.pump_next(registry).now_or_never() {
            None | Some(Ok(None)) => break,
            Some(Ok(Some(BufferDisposition::Closed(_)))) => {
                retire = true;
                break;
            }
            Some(Ok(Some(BufferDisposition::Coalesced))) => coalesced += 1,
            Some(Ok(Some(BufferDisposition::Degraded))) => degraded += 1,
            Some(Ok(Some(BufferDisposition::Queued))) => {}
            Some(Err(error)) => {
                if error.kind() == AsyncTransportErrorKind::Closed {
                    retire = true;
                }
                reconcile = true;
                break;
            }
        }
    }
    loop {
        let mut collector = FrameCollector {
            frames: Vec::new(),
            sse,
        };
        match document.dispatch_next(registry, &mut collector) {
            Ok(Some(AsyncDeliveryDisposition::Sequence(SequenceDisposition::Apply))) => {
                frames.extend(collector.frames);
            }
            Ok(Some(AsyncDeliveryDisposition::Sequence(
                SequenceDisposition::Degraded(_)
                | SequenceDisposition::AwaitingRecovery
                | SequenceDisposition::ScopeMismatch,
            ))) => {
                degraded += 1;
                reconcile = true;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(error) => match error.kind() {
                AsyncDeliveryErrorKind::Retired => {
                    retire = true;
                    break;
                }
                AsyncDeliveryErrorKind::AuthorizationLost | AsyncDeliveryErrorKind::Sequence(_) => {
                    reconcile = true
                }
            },
        }
        if frames.len() >= DELIVERY_BATCH {
            break;
        }
    }
    if reconcile {
        reconcile_memberships(state, key, document);
    }
    let metrics = (
        document.retained_events(),
        document.retained_bytes(),
        document.is_degraded(),
    );
    drop(guard);
    let mut tables = state.tables();
    if let Some(transport) = tables.transports.get_mut(key) {
        transport.coalesced = transport.coalesced.saturating_add(coalesced);
        transport.degraded_lanes = transport.degraded_lanes.saturating_add(degraded);
        transport.retained_events = metrics.0;
        transport.retained_bytes = metrics.1;
        transport.degraded = metrics.2;
    }
    (frames, retire)
}

/// Re-baselines degraded lanes at their delivery cursor and drops lost memberships.
fn reconcile_memberships(
    state: &AsyncState,
    key: &TransportKey,
    document: &mut BoundedDocumentTransportSession,
) {
    let members = {
        let tables = state.tables();
        let Some(transport) = tables.transports.get(key) else {
            return;
        };
        transport
            .memberships
            .iter()
            .filter_map(|id| {
                let record = tables.issued.get(id)?;
                let authorization = record.membership.clone()?;
                Some((
                    id.clone(),
                    authorization,
                    record.subscription.clone(),
                    Arc::clone(&record.delivery_cursor),
                    Arc::clone(&record.log),
                ))
            })
            .collect::<Vec<_>>()
    };
    let mut lost = Vec::new();
    for (id, authorization, subscription, cursor, log) in members {
        if !document.transport().contains_membership(&subscription) {
            lost.push(id);
            continue;
        }
        if document.sequence_state(&authorization)
            == Some(suprnova_live::async_updates::SequenceState::Degraded)
        {
            let position = cursor.load(Ordering::Acquire).saturating_sub(1);
            let epoch = lock_log(&log).epoch;
            let _ = document.recover_from_authoritative_refresh(
                &authorization,
                state.membership_registry.as_ref(),
                &FixedContinuity(StreamPosition::new(
                    StreamEpoch::new(epoch),
                    StreamSequence::new(position),
                )),
            );
        }
    }
    if lost.is_empty() {
        return;
    }
    let mut tables = state.tables();
    if let Some(transport) = tables.transports.get_mut(key) {
        for id in &lost {
            transport.memberships.remove(id);
        }
    }
    for id in lost {
        if let Some(record) = tables.issued.get_mut(&id) {
            record.membership = None;
            record.transport_wake = None;
        }
    }
}

pub(crate) fn browser_safe_generation(value: u64) -> bool {
    (1..=MAX_BROWSER_SAFE_INTEGER).contains(&value)
}
