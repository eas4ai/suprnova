//! Bounded Live v1 request, response, compatibility, and ordering contracts.

mod browser_context;
mod compatibility;
mod error;
mod idempotency;
mod limits;
mod ordering;
mod request;
mod response;
mod v2;

use crate::canonical::parse_canonical_value;

pub use browser_context::{BrowserRenderContext, DOCUMENT_KEY_EXTENSION_V1};
pub use compatibility::{CompatibilityDecision, CompatibilityWindow, VersionSet};
pub use error::{ProtocolError, ProtocolErrorKind};
pub use idempotency::{SemanticIdempotencyInputV1, semantic_idempotency_digest_v1};
pub use limits::{ProtocolLimitConfig, ProtocolLimits};
pub use ordering::{ApplicationStep, MorphDisposition, application_plan, application_plan_v2};
pub use request::{Operation, SnapshotInput, UpdateRequest, parse_update_request};
pub use response::{
    Emission, RenderPayload, ResponseOutcome, UpdateResponse, parse_update_response,
};
pub use v2::{ChildParameterDelivery, OperationV2, UpdateRequestV2, UpdateResponseV2, UrlIntent};

/// Fully parsed request dispatched by protocol version before its version-specific schema.
#[derive(Debug)]
pub enum VersionedUpdateRequest {
    /// Stable protocol-v1 model-sync/action request.
    V1(UpdateRequest),
    /// Protocol-v2 lifecycle-capable request.
    V2(UpdateRequestV2),
}

/// Fully parsed response dispatched by protocol version before its version-specific schema.
#[derive(Debug)]
pub enum VersionedUpdateResponse {
    /// Stable protocol-v1 response.
    V1(UpdateResponse),
    /// Protocol-v2 child/URL-capable response.
    V2(UpdateResponseV2),
}

/// Parses the protocol version first, then applies exactly one request schema.
pub fn parse_versioned_update_request(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<VersionedUpdateRequest, ProtocolError> {
    let value = parse_canonical_value(encoded, limits.input()).map_err(request::map_canonical)?;
    let fields = request::object(value)?;
    match request::protocol_version_from_fields(&fields)? {
        1 => request::parse_update_request_fields(fields, limits).map(VersionedUpdateRequest::V1),
        2 => v2::parse_update_request_v2_fields(fields, limits).map(VersionedUpdateRequest::V2),
        _ => Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion)),
    }
}

/// Parses the protocol version first, then applies exactly one response schema.
pub fn parse_versioned_update_response(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<VersionedUpdateResponse, ProtocolError> {
    let fields = v2::parse_response_envelope(encoded, limits)?;
    match request::protocol_version_from_fields(&fields)? {
        1 => {
            response::parse_update_response_fields(fields, limits).map(VersionedUpdateResponse::V1)
        }
        2 => v2::parse_update_response_v2_fields(fields, limits).map(VersionedUpdateResponse::V2),
        _ => Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion)),
    }
}

/// Canonically encodes one already validated versioned response.
pub fn encode_versioned_update_response(
    response: &VersionedUpdateResponse,
    limits: &ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    match response {
        VersionedUpdateResponse::V1(response) => response::encode_update_response(response, limits),
        VersionedUpdateResponse::V2(response) => v2::encode_update_response_v2(response, limits),
    }
}
