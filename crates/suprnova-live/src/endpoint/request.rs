//! Normalized method, media, body, and trusted-context input.

use std::fmt;

use bytes::Bytes;
use http::Method;

use crate::canonical::{CanonicalValue, parse_canonical_value};
use crate::host::MountSelection;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use crate::protocol::{
    ProtocolErrorKind, ProtocolLimits, SnapshotInput, VersionedUpdateRequest,
    parse_versioned_update_request,
};

use super::{EndpointError, EndpointErrorKind};

/// Exact protocol-v1 Live request and response media type.
pub const LIVE_MEDIA_TYPE_V1: &str = "application/vnd.suprnova.live+json; charset=utf-8; version=1";
/// Exact protocol-v2 Live request and response media type.
pub const LIVE_MEDIA_TYPE_V2: &str = "application/vnd.suprnova.live+json; charset=utf-8; version=2";

/// Parsed Live vendor media type with an independently comparable protocol version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedLiveMediaType {
    protocol_version: u16,
}

impl ParsedLiveMediaType {
    /// Parses the exact vendor type, UTF-8 charset, and supported version parameters.
    pub fn parse(value: &str) -> Result<Self, EndpointError> {
        let mut segments = value.split(';').map(str::trim);
        let media = segments.next().unwrap_or_default();
        if !media.eq_ignore_ascii_case("application/vnd.suprnova.live+json") {
            return Err(EndpointError::new(EndpointErrorKind::UnsupportedMediaType));
        }

        let mut charset = None;
        let mut version = None;
        for segment in segments {
            let Some((name, value)) = segment.split_once('=') else {
                return Err(EndpointError::new(EndpointErrorKind::UnsupportedMediaType));
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "charset" if charset.is_none() => charset = Some(value.trim()),
                "version" if version.is_none() => version = Some(value.trim()),
                _ => {
                    return Err(EndpointError::new(EndpointErrorKind::UnsupportedMediaType));
                }
            }
        }
        if !charset.is_some_and(|value| value.eq_ignore_ascii_case("utf-8")) {
            return Err(EndpointError::new(EndpointErrorKind::UnsupportedCharset));
        }
        let protocol_version = version
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or_else(|| EndpointError::new(EndpointErrorKind::UnsupportedVersion))?;
        if !matches!(protocol_version, 1 | 2) {
            return Err(EndpointError::new(EndpointErrorKind::UnsupportedVersion));
        }
        Ok(Self { protocol_version })
    }

    /// Returns the selected whole protocol version.
    #[must_use]
    pub const fn protocol_version(self) -> u16 {
        self.protocol_version
    }

    pub(crate) const fn response_value(self) -> &'static str {
        if self.protocol_version == 1 {
            LIVE_MEDIA_TYPE_V1
        } else {
            LIVE_MEDIA_TYPE_V2
        }
    }
}

impl fmt::Display for ParsedLiveMediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.response_value())
    }
}

/// Normalized result of checking request cache headers in the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestCachePolicy {
    /// The request explicitly bypasses caches as required.
    Bypass,
    /// A cacheable or cache-revalidation path was attempted.
    Attempted,
}

/// Host-neutral input accepted by the Live endpoint after adapter normalization.
pub struct LiveEndpointRequest {
    /// Normalized request method.
    pub method: Method,
    /// Parsed Live vendor media type.
    pub content_type: ParsedLiveMediaType,
    /// Complete bounded body bytes; no raw stream crosses this boundary.
    pub body: Bytes,
    /// Non-browser-constructible current host authority.
    pub context: TrustedLiveRequestContext,
    _normalized: NormalizedAdmission,
}

struct NormalizedAdmission;

impl LiveEndpointRequest {
    /// Admits only requests with trusted context and an explicit cache bypass decision.
    pub fn try_new(
        method: Method,
        content_type: ParsedLiveMediaType,
        body: Bytes,
        context: Option<TrustedLiveRequestContext>,
        cache_policy: RequestCachePolicy,
    ) -> Result<Self, EndpointError> {
        if cache_policy != RequestCachePolicy::Bypass {
            return Err(EndpointError::new(EndpointErrorKind::CacheAttempt));
        }
        let context =
            context.ok_or_else(|| EndpointError::new(EndpointErrorKind::MissingContext))?;
        Ok(Self {
            method,
            content_type,
            body,
            context,
            _normalized: NormalizedAdmission,
        })
    }
}

impl fmt::Debug for LiveEndpointRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveEndpointRequest")
            .field("method", &self.method)
            .field("content_type", &self.content_type)
            .field("body_bytes", &self.body.len())
            .field("context", &"<trusted:redacted>")
            .finish()
    }
}

pub(crate) fn inspect_mount(
    body: &[u8],
    media: ParsedLiveMediaType,
    limits: &ProtocolLimits,
) -> Result<MountSelection, EndpointError> {
    let request = parse_versioned_update_request(body, limits).map_err(|error| {
        EndpointError::new(match error.kind() {
            ProtocolErrorKind::UnsupportedVersion => EndpointErrorKind::UnsupportedVersion,
            ProtocolErrorKind::InputTooLarge | ProtocolErrorKind::SnapshotTooLarge => {
                EndpointErrorKind::RequestTooLarge
            }
            _ => EndpointErrorKind::MalformedProtocol,
        })
    })?;
    let (protocol, request_component, snapshot) = match &request {
        VersionedUpdateRequest::V1(request) => (
            request.protocol_version(),
            request.component(),
            request.snapshot(),
        ),
        VersionedUpdateRequest::V2(request) => (
            request.protocol_version(),
            request.component(),
            request.snapshot(),
        ),
    };
    if protocol != media.protocol_version() {
        return Err(EndpointError::new(EndpointErrorKind::UnsupportedVersion));
    }
    let envelope = match snapshot {
        SnapshotInput::Instance { envelope } | SnapshotInput::SeedPromotion { envelope, .. } => {
            envelope
        }
    };
    let (route, slot, component, contract_digest) = inspect_snapshot_identity(envelope, limits)?;
    if &component != request_component {
        return Err(EndpointError::new(EndpointErrorKind::ContextInconsistent));
    }
    Ok(MountSelection::new(
        route,
        slot,
        component,
        contract_digest,
        protocol,
    ))
}

fn inspect_snapshot_identity(
    envelope: &[u8],
    limits: &ProtocolLimits,
) -> Result<(RouteIdentity, IslandSlot, ComponentName, ContentDigest), EndpointError> {
    let envelope = parse_canonical_value(envelope, limits.input())
        .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
    let CanonicalValue::Object(envelope) = envelope else {
        return Err(EndpointError::new(EndpointErrorKind::MalformedProtocol));
    };
    let Some(CanonicalValue::Object(body)) = envelope.get("body") else {
        return Err(EndpointError::new(EndpointErrorKind::MalformedProtocol));
    };
    let Some(CanonicalValue::Object(component)) = body.get("component") else {
        return Err(EndpointError::new(EndpointErrorKind::MalformedProtocol));
    };
    let route = RouteIdentity::parse(string_field(body, "route")?)
        .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
    let slot = IslandSlot::parse(string_field(body, "slot")?)
        .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
    let component_name = ComponentName::parse(string_field(component, "name")?)
        .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
    let contract_digest = ContentDigest::parse(string_field(component, "contract_digest")?)
        .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
    Ok((route, slot, component_name, contract_digest))
}

fn string_field<'value>(
    fields: &'value std::collections::BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<&'value str, EndpointError> {
    let Some(CanonicalValue::String(value)) = fields.get(name) else {
        return Err(EndpointError::new(EndpointErrorKind::MalformedProtocol));
    };
    Ok(value)
}
