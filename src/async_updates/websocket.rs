//! Strict WebSocket origin policy and bounded canonical text frames.

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::limits::InputLimits;

use super::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeContext, AsyncEventSource, AsyncTransportError,
    AsyncTransportErrorKind, AuthorizedTransportAdd, AuthorizedTransportSubscription,
    DocumentTransportKind, DocumentTransportSession, EstablishingTransportAdd, PendingTransportAdd,
    PendingTransportRemove, ReadyTransportAdd, StreamName, SubscriptionBinding, SubscriptionId,
    VerifiedOrigin, decode_async_envelope, encode_async_envelope,
};

const MAX_WEBSOCKET_CONTROL_BYTES: usize = 512;
const MAX_WEBSOCKET_ENVELOPE_BYTES: usize = 65_536;
const MAX_WEBSOCKET_ORIGIN_ALLOWLIST: usize = 16;
const MAX_BROWSER_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// One complete incoming WebSocket message before async protocol decoding.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum WebSocketFrame<'a> {
    /// One text frame and whether it is the final fragment.
    Text {
        /// Raw UTF-8 candidate bytes.
        payload: &'a [u8],
        /// Whether this frame completes the message.
        final_fragment: bool,
    },
    /// A binary frame, unsupported by the JSON async contract.
    Binary(&'a [u8]),
    /// A continuation frame, rejected instead of reassembling unbounded fragments.
    Continuation(&'a [u8]),
}

impl std::fmt::Debug for WebSocketFrame<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text {
                payload,
                final_fragment,
            } => formatter
                .debug_struct("WebSocketFrame::Text")
                .field("bytes", &payload.len())
                .field("final_fragment", final_fragment)
                .finish(),
            Self::Binary(payload) => formatter
                .debug_tuple("WebSocketFrame::Binary")
                .field(&format_args!("{} bytes", payload.len()))
                .finish(),
            Self::Continuation(payload) => formatter
                .debug_tuple("WebSocketFrame::Continuation")
                .field(&format_args!("{} bytes", payload.len()))
                .finish(),
        }
    }
}

/// Exact bounded logical-membership control record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebSocketControlRecord {
    /// Request addition of an already descriptor-authorized subscription.
    Subscribe(SubscriptionId),
    /// Request removal of a current descriptor-authorized subscription.
    Unsubscribe(SubscriptionId),
}

/// Exact authenticated-membership request carried by one WebSocket control frame.
#[derive(Clone, Eq, PartialEq)]
pub struct WebSocketMembershipRequest {
    control_nonce: String,
    descriptor_binding: SubscriptionBinding,
    stream: StreamName,
    subscription: SubscriptionId,
    transport_generation: u64,
}

impl WebSocketMembershipRequest {
    /// Returns the one-connection control correlation nonce.
    #[must_use]
    pub fn control_nonce(&self) -> &str {
        &self.control_nonce
    }

    /// Returns the exact logical subscription identity.
    #[must_use]
    pub const fn subscription(&self) -> &SubscriptionId {
        &self.subscription
    }

    /// Returns the browser document transport generation.
    #[must_use]
    pub const fn transport_generation(&self) -> u64 {
        self.transport_generation
    }
}

impl std::fmt::Debug for WebSocketMembershipRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSocketMembershipRequest")
            .field("control_nonce", &self.control_nonce)
            .field("descriptor_binding", &"<redacted>")
            .field("stream", &self.stream)
            .field("subscription", &self.subscription)
            .field("transport_generation", &self.transport_generation)
            .finish()
    }
}

/// Post-commit proof for one exact WebSocket logical membership.
#[derive(Clone, Eq, PartialEq)]
pub struct WebSocketMembershipAcknowledgment {
    control_nonce: String,
    descriptor_binding: SubscriptionBinding,
    stream: StreamName,
    subscription: SubscriptionId,
    transport_generation: u64,
}

impl std::fmt::Debug for WebSocketMembershipAcknowledgment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSocketMembershipAcknowledgment")
            .field("control_nonce", &self.control_nonce)
            .field("descriptor_binding", &"<redacted>")
            .field("stream", &self.stream)
            .field("subscription", &self.subscription)
            .field("transport_generation", &self.transport_generation)
            .finish()
    }
}

/// One-use WebSocket membership request awaiting fresh transport authority.
#[must_use = "a pending WebSocket membership request must be authorized or dropped"]
pub struct PendingWebSocketMembershipAdd {
    request: WebSocketMembershipRequest,
    pending: PendingTransportAdd,
}

impl PendingWebSocketMembershipAdd {
    /// Revalidates exact current transport authority without borrowing the document.
    pub async fn authorize(self) -> Result<AuthorizedWebSocketMembershipAdd, AsyncTransportError> {
        let authorized = self.pending.authorize().await?;
        Ok(AuthorizedWebSocketMembershipAdd {
            request: self.request,
            authorized,
        })
    }
}

impl std::fmt::Debug for PendingWebSocketMembershipAdd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<PendingWebSocketMembershipAdd:redacted>")
    }
}

/// One-use authorized request awaiting a fresh document establishment snapshot.
#[must_use = "an authorized WebSocket membership must be established or dropped"]
pub struct AuthorizedWebSocketMembershipAdd {
    request: WebSocketMembershipRequest,
    authorized: AuthorizedTransportAdd,
}

impl AuthorizedWebSocketMembershipAdd {
    /// Rechecks document generation and membership fences before source work.
    pub fn prepare_establish(
        self,
        document: &DocumentTransportSession,
    ) -> Result<EstablishingWebSocketMembershipAdd, AsyncTransportError> {
        let establishing = document.prepare_establish(self.authorized)?;
        Ok(EstablishingWebSocketMembershipAdd {
            request: self.request,
            establishing,
        })
    }
}

impl std::fmt::Debug for AuthorizedWebSocketMembershipAdd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<AuthorizedWebSocketMembershipAdd:redacted>")
    }
}

/// One-use request authorized to establish its exact stream source.
#[must_use = "an establishing WebSocket membership must be established or dropped"]
pub struct EstablishingWebSocketMembershipAdd {
    request: WebSocketMembershipRequest,
    establishing: EstablishingTransportAdd,
}

impl EstablishingWebSocketMembershipAdd {
    /// Establishes the source and repeats current authority validation after the await.
    pub async fn establish(
        self,
        source: &dyn AsyncEventSource,
    ) -> Result<ReadyWebSocketMembershipAdd, AsyncTransportError> {
        let ready = self.establishing.establish(source).await?;
        Ok(ReadyWebSocketMembershipAdd {
            request: self.request,
            ready,
        })
    }
}

impl std::fmt::Debug for EstablishingWebSocketMembershipAdd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<EstablishingWebSocketMembershipAdd:redacted>")
    }
}

/// One-use exact membership ready for synchronous document commit.
#[must_use = "a ready WebSocket membership must be committed or dropped"]
pub struct ReadyWebSocketMembershipAdd {
    request: WebSocketMembershipRequest,
    ready: ReadyTransportAdd,
}

impl std::fmt::Debug for ReadyWebSocketMembershipAdd {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<ReadyWebSocketMembershipAdd:redacted>")
    }
}

/// Non-cloneable one-use receipt emitted by one exact successful membership commit.
///
/// ```compile_fail
/// use suprnova_live::async_updates::WebSocketMembershipCommitReceipt;
///
/// fn clone_receipt(receipt: WebSocketMembershipCommitReceipt) {
///     let _duplicate = receipt.clone();
/// }
/// ```
///
/// ```compile_fail
/// use suprnova_live::async_updates::{
///     WebSocketMembershipCommitReceipt, WebSocketMembershipControl,
/// };
///
/// fn reuse_receipt(receipt: WebSocketMembershipCommitReceipt) {
///     let _first = WebSocketMembershipControl::acknowledge_committed(receipt);
///     let _second = WebSocketMembershipControl::acknowledge_committed(receipt);
/// }
/// ```
#[must_use = "a WebSocket membership commit receipt must be acknowledged or dropped"]
pub struct WebSocketMembershipCommitReceipt {
    request: WebSocketMembershipRequest,
}

impl std::fmt::Debug for WebSocketMembershipCommitReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<WebSocketMembershipCommitReceipt:redacted>")
    }
}

/// Authenticated application of decoded membership controls.
pub struct WebSocketMembershipControl;

impl WebSocketMembershipControl {
    /// Prepares an exact descriptor-, stream-, and request-bound subscription.
    pub fn prepare_authenticated_subscribe(
        document: &DocumentTransportSession,
        request: WebSocketMembershipRequest,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<PendingWebSocketMembershipAdd, AsyncTransportError> {
        if request.subscription != *authorization.subscription()
            || request.descriptor_binding != *authorization.binding()
            || request.stream != *authorization.context().stream()
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::RoutingMismatch,
            ));
        }
        let pending = Self::prepare_subscribe(
            document,
            &WebSocketControlRecord::Subscribe(request.subscription.clone()),
            authorization,
        )?;
        Ok(PendingWebSocketMembershipAdd { request, pending })
    }

    /// Commits one exact membership and emits its non-cloneable receipt.
    pub fn commit_authenticated_subscribe(
        document: &mut DocumentTransportSession,
        ready: ReadyWebSocketMembershipAdd,
    ) -> Result<WebSocketMembershipCommitReceipt, AsyncTransportError> {
        validate_document_kind(document)?;
        document.commit_add(ready.ready)?;
        Ok(WebSocketMembershipCommitReceipt {
            request: ready.request,
        })
    }

    /// Consumes one exact commit receipt to mint one membership acknowledgment.
    #[must_use]
    pub fn acknowledge_committed(
        receipt: WebSocketMembershipCommitReceipt,
    ) -> WebSocketMembershipAcknowledgment {
        let request = receipt.request;
        WebSocketMembershipAcknowledgment {
            control_nonce: request.control_nonce,
            descriptor_binding: request.descriptor_binding,
            stream: request.stream,
            subscription: request.subscription,
            transport_generation: request.transport_generation,
        }
    }

    /// Prepares the exact subscription named by a verified subscribe record.
    pub fn prepare_subscribe(
        document: &DocumentTransportSession,
        control: &WebSocketControlRecord,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<PendingTransportAdd, AsyncTransportError> {
        validate_document_kind(document)?;
        match control {
            WebSocketControlRecord::Subscribe(subscription)
                if subscription == authorization.subscription() =>
            {
                document.prepare_add(authorization)
            }
            WebSocketControlRecord::Subscribe(_) | WebSocketControlRecord::Unsubscribe(_) => Err(
                AsyncTransportError::new(AsyncTransportErrorKind::RoutingMismatch),
            ),
        }
    }

    /// Prepares authenticated removal for the exact unsubscribe record.
    pub fn prepare_unsubscribe<'a>(
        document: &DocumentTransportSession,
        control: &WebSocketControlRecord,
        authorization: &'a AuthorizedTransportSubscription,
    ) -> Result<PendingTransportRemove<'a>, AsyncTransportError> {
        validate_document_kind(document)?;
        match control {
            WebSocketControlRecord::Unsubscribe(subscription)
                if subscription == authorization.subscription() =>
            {
                document.prepare_remove(authorization)
            }
            WebSocketControlRecord::Subscribe(_) | WebSocketControlRecord::Unsubscribe(_) => Err(
                AsyncTransportError::new(AsyncTransportErrorKind::RoutingMismatch),
            ),
        }
    }
}

fn validate_document_kind(document: &DocumentTransportSession) -> Result<(), AsyncTransportError> {
    if document.kind() != DocumentTransportKind::WebSocket {
        return Err(AsyncTransportError::new(
            AsyncTransportErrorKind::TransportMismatch,
        ));
    }
    Ok(())
}

impl WebSocketControlRecord {
    /// Returns the logical routing identity.
    #[must_use]
    pub const fn subscription(&self) -> &SubscriptionId {
        match self {
            Self::Subscribe(subscription) | Self::Unsubscribe(subscription) => subscription,
        }
    }
}

/// Authentication result produced only after strict origin validation succeeds.
pub enum WebSocketAuthentication<T> {
    /// Same-origin session-cookie authority.
    Cookie(T),
    /// Separately verified non-cookie authority required for cross-origin use.
    SeparateCredential(T),
}

/// WebSocket upgrade result with exact normalized origin and typed host authority.
pub struct AuthorizedWebSocketUpgrade<T> {
    origin: VerifiedOrigin,
    cross_origin: bool,
    authority: T,
}

impl<T> AuthorizedWebSocketUpgrade<T> {
    /// Returns the exact normalized browser origin.
    #[must_use]
    pub const fn origin(&self) -> &VerifiedOrigin {
        &self.origin
    }

    /// Returns whether explicit cross-origin policy was used.
    #[must_use]
    pub const fn is_cross_origin(&self) -> bool {
        self.cross_origin
    }

    /// Consumes the upgrade proof and returns host authentication authority.
    #[must_use]
    pub fn into_authority(self) -> T {
        self.authority
    }
}

impl<T> std::fmt::Debug for AuthorizedWebSocketUpgrade<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedWebSocketUpgrade")
            .field("origin", &self.origin)
            .field("cross_origin", &self.cross_origin)
            .field("authority", &"<redacted>")
            .finish()
    }
}

/// Exact application origin plus a finite non-wildcard cross-origin allowlist.
pub struct WebSocketOriginPolicy {
    application: VerifiedOrigin,
    allowed_cross_origins: Vec<VerifiedOrigin>,
}

impl WebSocketOriginPolicy {
    /// Creates a bounded exact-origin policy without duplicate entries.
    pub fn new(
        application: VerifiedOrigin,
        mut allowed_cross_origins: Vec<VerifiedOrigin>,
    ) -> Result<Self, AsyncTransportError> {
        if allowed_cross_origins.len() > MAX_WEBSOCKET_ORIGIN_ALLOWLIST {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidOrigin,
            ));
        }
        allowed_cross_origins.sort_by_key(ToString::to_string);
        if allowed_cross_origins
            .windows(2)
            .any(|pair| pair[0] == pair[1])
            || allowed_cross_origins
                .iter()
                .any(|origin| origin == &application)
        {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidOrigin,
            ));
        }
        Ok(Self {
            application,
            allowed_cross_origins,
        })
    }

    /// Validates origin before invoking host descriptor or credential processing.
    pub fn authorize_upgrade<T>(
        &self,
        origin_headers: &[&str],
        authenticate: impl FnOnce() -> Result<WebSocketAuthentication<T>, AsyncTransportError>,
    ) -> Result<AuthorizedWebSocketUpgrade<T>, AsyncTransportError> {
        let [origin_header] = origin_headers else {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidOrigin,
            ));
        };
        let origin = VerifiedOrigin::parse(origin_header)?;
        let cross_origin = origin != self.application;
        if cross_origin && !self.allowed_cross_origins.contains(&origin) {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidOrigin,
            ));
        }
        let authority = match authenticate()? {
            WebSocketAuthentication::Cookie(authority) if cross_origin => {
                let _ = authority;
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::AuthorizationScopeMismatch,
                ));
            }
            WebSocketAuthentication::Cookie(authority)
            | WebSocketAuthentication::SeparateCredential(authority) => authority,
        };
        Ok(AuthorizedWebSocketUpgrade {
            origin,
            cross_origin,
            authority,
        })
    }
}

/// Bounded canonical WebSocket text-frame codec.
pub struct WebSocketCodec {
    envelope_limits: AsyncCodecLimits,
}

impl WebSocketCodec {
    /// Returns the independently versioned async-envelope v1 codec.
    #[must_use]
    pub fn v1() -> Self {
        Self {
            envelope_limits: AsyncCodecLimits::v1(),
        }
    }

    /// Encodes an authorized envelope as one canonical text-message payload.
    pub fn encode_envelope(
        &self,
        envelope: &AsyncEnvelope,
    ) -> Result<Vec<u8>, AsyncTransportError> {
        encode_async_envelope(envelope, &self.envelope_limits)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))
    }

    /// Decodes one complete canonical text message under exact membership context.
    pub fn decode_envelope(
        &self,
        frame: WebSocketFrame<'_>,
        context: &AsyncEnvelopeContext,
    ) -> Result<AsyncEnvelope, AsyncTransportError> {
        let payload = text_payload(frame, MAX_WEBSOCKET_ENVELOPE_BYTES)?;
        decode_async_envelope(payload, &self.envelope_limits, context)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))
    }

    /// Encodes one exact-key bounded subscribe or unsubscribe record.
    pub fn encode_control(
        &self,
        control: &WebSocketControlRecord,
    ) -> Result<Vec<u8>, AsyncTransportError> {
        let wire = ControlWire::from_control(control);
        let value = serde_json::to_value(wire).map_err(|_| invalid_envelope())?;
        let canonical = CanonicalValue::from_serde_value(value).map_err(|_| invalid_envelope())?;
        to_canonical_bytes(&canonical, &control_limits()?).map_err(|_| invalid_envelope())
    }

    /// Decodes exact control fields without consulting document membership state.
    ///
    /// Every syntactically valid identity continues to fresh authorization;
    /// only the later synchronous commit may classify local membership state.
    pub fn decode_control(
        &self,
        frame: WebSocketFrame<'_>,
    ) -> Result<WebSocketControlRecord, AsyncTransportError> {
        let payload = text_payload(frame, MAX_WEBSOCKET_CONTROL_BYTES)?;
        let limits = control_limits()?;
        let canonical = parse_canonical_value(payload, &limits).map_err(|_| invalid_envelope())?;
        let recoded = to_canonical_bytes(&canonical, &limits).map_err(|_| invalid_envelope())?;
        if recoded != payload {
            return Err(invalid_envelope());
        }
        let value = canonical.to_serde_value().map_err(|_| invalid_envelope())?;
        let wire: ControlWire = serde_json::from_value(value).map_err(|_| invalid_envelope())?;
        wire.into_control()
    }

    /// Decodes one canonical exact-membership request without granting authority.
    pub fn decode_membership_request(
        &self,
        frame: WebSocketFrame<'_>,
    ) -> Result<WebSocketMembershipRequest, AsyncTransportError> {
        let payload = text_payload(frame, MAX_WEBSOCKET_CONTROL_BYTES)?;
        let limits = control_limits()?;
        let canonical = parse_canonical_value(payload, &limits).map_err(|_| invalid_envelope())?;
        let recoded = to_canonical_bytes(&canonical, &limits).map_err(|_| invalid_envelope())?;
        if recoded != payload {
            return Err(invalid_envelope());
        }
        let value = canonical.to_serde_value().map_err(|_| invalid_envelope())?;
        let wire: MembershipRequestWire =
            serde_json::from_value(value).map_err(|_| invalid_envelope())?;
        wire.into_request()
    }

    /// Encodes one post-commit exact-membership acknowledgment.
    pub fn encode_membership_acknowledgment(
        &self,
        acknowledgment: &WebSocketMembershipAcknowledgment,
    ) -> Result<Vec<u8>, AsyncTransportError> {
        let value = serde_json::to_value(MembershipAcknowledgmentWire::from_acknowledgment(
            acknowledgment,
        ))
        .map_err(|_| invalid_envelope())?;
        let canonical = CanonicalValue::from_serde_value(value).map_err(|_| invalid_envelope())?;
        to_canonical_bytes(&canonical, &control_limits()?).map_err(|_| invalid_envelope())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipRequestWire {
    control_nonce: String,
    descriptor_binding: String,
    kind: String,
    stream: String,
    subscription: String,
    transport_generation: u64,
}

impl MembershipRequestWire {
    fn into_request(self) -> Result<WebSocketMembershipRequest, AsyncTransportError> {
        if self.kind != "subscribe"
            || !valid_control_nonce(&self.control_nonce)
            || self.transport_generation == 0
            || self.transport_generation > MAX_BROWSER_SAFE_INTEGER
        {
            return Err(invalid_envelope());
        }
        Ok(WebSocketMembershipRequest {
            control_nonce: self.control_nonce,
            descriptor_binding: SubscriptionBinding::parse(&self.descriptor_binding)
                .map_err(|_| invalid_envelope())?,
            stream: StreamName::parse(&self.stream).map_err(|_| invalid_envelope())?,
            subscription: SubscriptionId::parse(&self.subscription)
                .map_err(|_| invalid_envelope())?,
            transport_generation: self.transport_generation,
        })
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct MembershipAcknowledgmentWire {
    control_nonce: String,
    descriptor_binding: String,
    kind: String,
    stream: String,
    subscription: String,
    transport_generation: u64,
}

impl MembershipAcknowledgmentWire {
    fn from_acknowledgment(acknowledgment: &WebSocketMembershipAcknowledgment) -> Self {
        Self {
            control_nonce: acknowledgment.control_nonce.clone(),
            descriptor_binding: acknowledgment.descriptor_binding.to_base64url(),
            kind: "membership_authenticated".to_owned(),
            stream: acknowledgment.stream.as_str().to_owned(),
            subscription: acknowledgment.subscription.to_base64url(),
            transport_generation: acknowledgment.transport_generation,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ControlWire {
    kind: String,
    subscription: String,
}

impl ControlWire {
    fn from_control(control: &WebSocketControlRecord) -> Self {
        let kind = match control {
            WebSocketControlRecord::Subscribe(_) => "subscribe",
            WebSocketControlRecord::Unsubscribe(_) => "unsubscribe",
        };
        Self {
            kind: kind.to_owned(),
            subscription: control.subscription().to_base64url(),
        }
    }

    fn into_control(self) -> Result<WebSocketControlRecord, AsyncTransportError> {
        let subscription = SubscriptionId::parse(&self.subscription)
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        match self.kind.as_str() {
            "subscribe" => Ok(WebSocketControlRecord::Subscribe(subscription)),
            "unsubscribe" => Ok(WebSocketControlRecord::Unsubscribe(subscription)),
            _ => Err(invalid_envelope()),
        }
    }
}

fn text_payload(
    frame: WebSocketFrame<'_>,
    maximum_bytes: usize,
) -> Result<&[u8], AsyncTransportError> {
    match frame {
        WebSocketFrame::Text {
            payload,
            final_fragment: true,
        } => {
            if payload.len() > maximum_bytes {
                return Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::FrameTooLarge,
                ));
            }
            std::str::from_utf8(payload)
                .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::UnsupportedFrame))?;
            Ok(payload)
        }
        WebSocketFrame::Text { .. }
        | WebSocketFrame::Binary(_)
        | WebSocketFrame::Continuation(_) => Err(AsyncTransportError::new(
            AsyncTransportErrorKind::UnsupportedFrame,
        )),
    }
}

fn control_limits() -> Result<InputLimits, AsyncTransportError> {
    InputLimits::new(MAX_WEBSOCKET_CONTROL_BYTES, 3, 8, 256)
        .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))
}

fn valid_control_nonce(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
}

fn invalid_envelope() -> AsyncTransportError {
    AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope)
}
