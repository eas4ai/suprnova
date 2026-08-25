//! Strict WebSocket origin policy and bounded canonical text frames.

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::limits::InputLimits;

use super::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeContext, AsyncEventSource, AsyncTransportError,
    AsyncTransportErrorKind, AuthorizedTransportSubscription, CloseDisposition,
    DocumentTransportKind, DocumentTransportSession, SubscriptionId, VerifiedOrigin,
    decode_async_envelope, encode_async_envelope,
};

const MAX_WEBSOCKET_CONTROL_BYTES: usize = 512;
const MAX_WEBSOCKET_ENVELOPE_BYTES: usize = 65_536;
const MAX_WEBSOCKET_ORIGIN_ALLOWLIST: usize = 16;

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

/// Authenticated application of decoded membership controls.
pub struct WebSocketMembershipControl;

impl WebSocketMembershipControl {
    /// Adds the exact subscription named by a verified subscribe record.
    pub async fn subscribe(
        document: &mut DocumentTransportSession,
        control: &WebSocketControlRecord,
        source: &dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<(), AsyncTransportError> {
        validate_document_kind(document)?;
        match control {
            WebSocketControlRecord::Subscribe(subscription)
                if subscription == authorization.subscription() =>
            {
                document.add(source, authorization).await
            }
            WebSocketControlRecord::Subscribe(_) | WebSocketControlRecord::Unsubscribe(_) => Err(
                AsyncTransportError::new(AsyncTransportErrorKind::RoutingMismatch),
            ),
        }
    }

    /// Removes the exact subscription named by a verified unsubscribe record.
    pub async fn unsubscribe(
        document: &mut DocumentTransportSession,
        control: &WebSocketControlRecord,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<CloseDisposition, AsyncTransportError> {
        validate_document_kind(document)?;
        match control {
            WebSocketControlRecord::Unsubscribe(subscription)
                if subscription == authorization.subscription() =>
            {
                document.remove(authorization).await
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

    /// Decodes exact control fields and enforces current membership expectations.
    pub fn decode_control(
        &self,
        frame: WebSocketFrame<'_>,
        document: &DocumentTransportSession,
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
        let control = wire.into_control()?;
        match &control {
            WebSocketControlRecord::Subscribe(subscription)
                if document.contains_membership(subscription) =>
            {
                Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::DuplicateMembership,
                ))
            }
            WebSocketControlRecord::Unsubscribe(subscription)
                if !document.contains_membership(subscription) =>
            {
                Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::UnknownMembership,
                ))
            }
            _ => Ok(control),
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
    InputLimits::new(MAX_WEBSOCKET_CONTROL_BYTES, 3, 4, 256)
        .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))
}

fn invalid_envelope() -> AsyncTransportError {
    AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope)
}
