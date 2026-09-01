//! Closed, payload-free endpoint and kernel failures.

use std::error::Error;
use std::fmt;

/// Stable failure categories at the normalized Live HTTP boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointErrorKind {
    /// The host adapter did not supply a trusted request capability.
    MissingContext,
    /// A cacheable request path was attempted for the mutable Live endpoint.
    CacheAttempt,
    /// Only `POST` is admitted.
    MethodNotAllowed,
    /// The media type was not the registered Live vendor type.
    UnsupportedMediaType,
    /// The Live JSON charset was missing or not UTF-8.
    UnsupportedCharset,
    /// The media or protocol version is unsupported or inconsistent.
    UnsupportedVersion,
    /// Whole request bytes exceeded the configured ceiling.
    RequestTooLarge,
    /// Protocol parsing or batch validation failed.
    MalformedProtocol,
    /// Trusted host facts expired before admission.
    ContextExpired,
    /// Trusted host, catalog, request, or snapshot facts disagreed.
    ContextInconsistent,
    /// Immutable registry lookup did not match the trusted catalog contract.
    RegistryMismatch,
    /// Signed snapshot authority was invalid, expired, or bound elsewhere.
    SnapshotRejected,
    /// A kernel response was absent, malformed, mismatched, or unsafe.
    InvalidKernelResponse,
    /// Complete response bytes exceeded the configured ceiling.
    ResponseTooLarge,
    /// The application kernel failed without a safe protocol outcome.
    KernelUnavailable,
    /// The endpoint clock could not provide current time.
    ClockUnavailable,
    /// Endpoint configuration violated an invariant.
    InvalidConfiguration,
}

/// Redacted endpoint failure that never retains request or response payloads.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EndpointError {
    kind: EndpointErrorKind,
}

impl EndpointError {
    pub(crate) const fn new(kind: EndpointErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> EndpointErrorKind {
        self.kind
    }
}

impl fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            EndpointErrorKind::MissingContext => "live_endpoint_missing_context",
            EndpointErrorKind::CacheAttempt => "live_endpoint_cache_attempt",
            EndpointErrorKind::MethodNotAllowed => "live_endpoint_method_not_allowed",
            EndpointErrorKind::UnsupportedMediaType => "live_endpoint_unsupported_media_type",
            EndpointErrorKind::UnsupportedCharset => "live_endpoint_unsupported_charset",
            EndpointErrorKind::UnsupportedVersion => "live_endpoint_unsupported_version",
            EndpointErrorKind::RequestTooLarge => "live_endpoint_request_too_large",
            EndpointErrorKind::MalformedProtocol => "live_endpoint_malformed_protocol",
            EndpointErrorKind::ContextExpired => "live_endpoint_context_expired",
            EndpointErrorKind::ContextInconsistent => "live_endpoint_context_inconsistent",
            EndpointErrorKind::RegistryMismatch => "live_endpoint_registry_mismatch",
            EndpointErrorKind::SnapshotRejected => "live_endpoint_snapshot_rejected",
            EndpointErrorKind::InvalidKernelResponse => "live_endpoint_invalid_kernel_response",
            EndpointErrorKind::ResponseTooLarge => "live_endpoint_response_too_large",
            EndpointErrorKind::KernelUnavailable => "live_endpoint_kernel_unavailable",
            EndpointErrorKind::ClockUnavailable => "live_endpoint_clock_unavailable",
            EndpointErrorKind::InvalidConfiguration => "live_endpoint_invalid_configuration",
        })
    }
}

impl fmt::Debug for EndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for EndpointError {}

/// Closed failure returned by the application-facing endpoint kernel.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct EndpointKernelError {
    kind: EndpointErrorKind,
}

impl EndpointKernelError {
    /// Creates a payload-free kernel availability failure.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            kind: EndpointErrorKind::KernelUnavailable,
        }
    }

    /// Creates a payload-free trusted-context inconsistency failure.
    #[must_use]
    pub const fn context_inconsistent() -> Self {
        Self {
            kind: EndpointErrorKind::ContextInconsistent,
        }
    }

    pub(crate) const fn kind(self) -> EndpointErrorKind {
        self.kind
    }
}

impl fmt::Display for EndpointKernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            EndpointErrorKind::ContextInconsistent => "live_endpoint_context_inconsistent",
            _ => "live_endpoint_kernel_unavailable",
        })
    }
}

impl fmt::Debug for EndpointKernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for EndpointKernelError {}
