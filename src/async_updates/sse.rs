//! Server-Sent Events wire encoding and same-origin membership control.

use http::HeaderMap;
use http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderName, HeaderValue, X_CONTENT_TYPE_OPTIONS};

use super::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEventSource, AsyncTransportError,
    AsyncTransportErrorKind, AuthorizedTransportSubscription, CloseDisposition,
    DocumentTransportHandle, DocumentTransportKind, DocumentTransportSession, VerifiedOrigin,
    encode_async_envelope,
};

const SSE_EVENT_NAME: &str = "suprnova-live-async";
const HEARTBEAT_COMMENT: &[u8] = b": suprnova-live heartbeat\n\n";

/// One bounded SSE event whose `data` bytes are the canonical Task 3 envelope.
pub struct SseEvent {
    id: String,
    data: Vec<u8>,
    encoded: Vec<u8>,
}

impl SseEvent {
    /// Returns the bounded non-authoritative correlation identifier.
    ///
    /// Native `Last-Event-ID` replay is never continuity authority for a
    /// multiplexed document; every logical subscription retains its own Task 3
    /// baseline and sequence machine.
    #[must_use]
    pub const fn id(&self) -> &str {
        self.id.as_str()
    }

    /// Returns the fixed asynchronous event name.
    #[must_use]
    pub const fn event(&self) -> &'static str {
        SSE_EVENT_NAME
    }

    /// Returns the canonical Task 3 envelope bytes without the SSE field prefix.
    #[must_use]
    pub const fn data(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Returns the complete SSE record ending in one empty line.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        self.encoded.as_slice()
    }
}

impl std::fmt::Debug for SseEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SseEvent")
            .field("id", &self.id)
            .field("data_bytes", &self.data.len())
            .field("encoded_bytes", &self.encoded.len())
            .finish()
    }
}

/// Bounded canonical SSE event encoder.
pub struct SseEncoder;

impl SseEncoder {
    /// Encodes one bounded Task 3 envelope without any user-controlled SSE line breaks.
    pub fn encode_envelope(envelope: &AsyncEnvelope) -> Result<SseEvent, AsyncTransportError> {
        let data = encode_async_envelope(envelope, &AsyncCodecLimits::v1())
            .map_err(|_| AsyncTransportError::new(AsyncTransportErrorKind::InvalidEnvelope))?;
        if data.contains(&b'\n') || data.contains(&b'\r') {
            return Err(AsyncTransportError::new(
                AsyncTransportErrorKind::InvalidEnvelope,
            ));
        }
        let position = envelope.position();
        let id = format!(
            "{}/{}/{}",
            envelope.subscription().to_base64url(),
            position.epoch().get(),
            position.sequence().get()
        );
        let mut encoded = Vec::with_capacity(id.len() + data.len() + 64);
        encoded.extend_from_slice(b"id:");
        encoded.extend_from_slice(id.as_bytes());
        encoded.extend_from_slice(b"\nevent:");
        encoded.extend_from_slice(SSE_EVENT_NAME.as_bytes());
        encoded.extend_from_slice(b"\ndata:");
        encoded.extend_from_slice(&data);
        encoded.extend_from_slice(b"\n\n");
        Ok(SseEvent { id, data, encoded })
    }

    /// Returns the fixed connection-liveness comment record.
    #[must_use]
    pub const fn heartbeat_comment() -> &'static [u8] {
        HEARTBEAT_COMMENT
    }
}

/// Exact response metadata required for an authorized SSE stream.
pub struct SseResponseContract;

impl SseResponseContract {
    /// Returns non-cacheable, non-buffered, non-sniffable SSE response headers.
    #[must_use]
    pub fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store, no-transform"),
        );
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        );
        headers
    }
}

/// Authenticated same-origin membership changes around a correlation-only handle.
pub struct SseMembershipControl;

impl SseMembershipControl {
    /// Adds one currently authorized logical membership to the document stream.
    pub async fn subscribe(
        document: &mut DocumentTransportSession,
        handle: &DocumentTransportHandle,
        origin: &VerifiedOrigin,
        source: &dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
    ) -> Result<(), AsyncTransportError> {
        validate_control(document, handle, origin)?;
        document.add(source, authorization).await
    }

    /// Removes one logical membership only with matching current authorization.
    pub async fn unsubscribe(
        document: &mut DocumentTransportSession,
        handle: &DocumentTransportHandle,
        origin: &VerifiedOrigin,
        authorization: &AuthorizedTransportSubscription,
    ) -> Result<CloseDisposition, AsyncTransportError> {
        validate_control(document, handle, origin)?;
        document.remove(authorization).await
    }
}

fn validate_control(
    document: &DocumentTransportSession,
    handle: &DocumentTransportHandle,
    origin: &VerifiedOrigin,
) -> Result<(), AsyncTransportError> {
    if document.kind() != DocumentTransportKind::ServerSentEvents {
        return Err(AsyncTransportError::new(
            AsyncTransportErrorKind::TransportMismatch,
        ));
    }
    if document.handle() != handle {
        return Err(AsyncTransportError::new(
            AsyncTransportErrorKind::RoutingMismatch,
        ));
    }
    if document.origin() != origin {
        return Err(AsyncTransportError::new(
            AsyncTransportErrorKind::OriginMismatch,
        ));
    }
    Ok(())
}
