//! Typed kernel disposition and complete HTTP response intent.

use bytes::Bytes;
use http::{HeaderMap, StatusCode};

/// Closed semantic disposition returned by the endpoint kernel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointOutcomeKind {
    /// A new committed outcome is returned.
    Accepted,
    /// A retained prior committed outcome is returned without re-execution.
    Duplicate,
    /// Validation or ordinary request policy rejected the operation.
    Rejected,
    /// Authentication or authorization failed under resource-concealment policy.
    Concealed,
    /// Revision or idempotency authority conflicted with this request.
    Conflict,
    /// Browser authority must fresh-render without replaying the operation.
    RefreshRequired,
    /// Live processing cannot safely continue for the island.
    Fatal,
}

impl EndpointOutcomeKind {
    pub(crate) const fn status(self) -> StatusCode {
        match self {
            Self::Accepted | Self::Duplicate => StatusCode::OK,
            Self::Rejected => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Concealed => StatusCode::NOT_FOUND,
            Self::Conflict | Self::RefreshRequired => StatusCode::CONFLICT,
            Self::Fatal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Complete protocol bytes paired with their closed semantic HTTP disposition.
pub struct EndpointDispatch {
    pub(crate) outcome: EndpointOutcomeKind,
    pub(crate) body: Bytes,
}

impl EndpointDispatch {
    /// Creates a kernel result pending endpoint validation and canonical re-encoding.
    #[must_use]
    pub const fn new(outcome: EndpointOutcomeKind, body: Bytes) -> Self {
        Self { outcome, body }
    }
}

impl std::fmt::Debug for EndpointDispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EndpointDispatch")
            .field("outcome", &self.outcome)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Complete host-neutral HTTP response intent for the Suprnova adapter.
pub struct LiveEndpointResponse {
    /// Exact status selected by the endpoint's closed mapping.
    pub status: StatusCode,
    /// Endpoint-owned cache, media, length, and bounded security headers.
    pub headers: HeaderMap,
    /// Fully encoded bytes; partial protocol output is never represented.
    pub body: Bytes,
}

impl std::fmt::Debug for LiveEndpointResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveEndpointResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}
