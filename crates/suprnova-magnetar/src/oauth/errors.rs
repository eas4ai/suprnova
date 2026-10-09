//! Errors raised by the I/O-free OAuth protocol and request-shape types.
//!
//! This module maps protocol-shape failures into the engine-wide error-class
//! contract (`docs/specs/suprnova-magnetar/09-oauth-engine.md`) and exposes a
//! redacted tracing envelope for non-secret observability.

use core::fmt;

/// The result type used by the OAuth protocol and request-shape modules.
pub type OAuthResult<T> = core::result::Result<T, OAuthProtocolError>;

/// Status class for OAuth protocol failures, used by the engine-wide error
/// contract in `docs/specs/suprnova-magnetar/09-oauth-engine.md`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OAuthErrorClass {
    /// 400-class: malformed client request, malformed provider response, or
    /// provider-reported OAuth error codes that indicate caller misinput.
    ClientError,
    /// 401-class: failed or unverifiable provider identity proof.
    IdentityError,
    /// 502-class: transient upstream/provider transport or 5xx failures.
    UpstreamError,
    /// 500-class: Magnetar-side misconfiguration or implementation fault.
    ServerError,
}

impl OAuthErrorClass {
    /// The protocol HTTP status for this class.
    pub const fn status(self) -> u16 {
        match self {
            Self::ClientError => 400,
            Self::IdentityError => 401,
            Self::UpstreamError => 502,
            Self::ServerError => 500,
        }
    }
}

/// A redacted tracing envelope for OAuth errors.
#[derive(Debug, PartialEq, Eq)]
pub struct OAuthErrorTraceContext<'a> {
    /// The mapped failure class.
    pub class: OAuthErrorClass,
    /// Provider name when known.
    pub provider: Option<&'static str>,
    /// The grant under which the error arose.
    pub grant: &'static str,
    /// The ceremony kind bound to the failed flow.
    pub ceremony_kind: &'static str,
    /// A non-secret correlation identifier chosen by the caller.
    pub correlation_id: &'a str,
}

/// Error values raised by OAuth protocol and request-shape code.
#[derive(Clone, PartialEq, Eq)]
pub enum OAuthProtocolError {
    /// A malformed OAuth request shape (for example, missing PKCE or nonce).
    InvalidRequestShape {
        /// The request field that failed to validate.
        field: String,
        /// Detail for diagnostics. Not emitted in `Display`/`Debug`.
        message: String,
    },
    /// A token response was malformed relative to the expected wire shape.
    MalformedTokenResponse {
        /// Detail for diagnostics. Not emitted in `Display`/`Debug`.
        message: String,
    },
    /// A provider-specific response body was malformed for identity resolution.
    MalformedProviderResponse {
        /// Normalized provider name.
        provider: &'static str,
        /// Detail for diagnostics. Not emitted in `Display`/`Debug`.
        message: String,
    },
    /// A provider returned an OAuth-style error payload.
    ProviderReportedError {
        /// Normalized provider name.
        provider: &'static str,
        /// OAuth error code returned by the provider.
        code: String,
        /// Provider-provided error description, if any. Not emitted in
        /// `Display`/`Debug`.
        message: Option<String>,
    },
    /// A provider identity proof could not be verified.
    IdentityVerificationFailed {
        /// Normalized provider name.
        provider: &'static str,
        /// Non-secret description of why verification failed.
        reason: String,
    },
    /// A provider/network-level problem that may be retriable.
    UpstreamUnavailable {
        /// Normalized provider name.
        provider: &'static str,
        /// Detail for diagnostics. Not emitted in `Display`/`Debug`.
        message: String,
        /// Optional `Retry-After` value from the response, where known.
        retry_after_seconds: Option<u64>,
    },
    /// A provider registration, config, or implementation problem.
    ProviderConfiguration {
        /// Normalized provider name.
        provider: &'static str,
        /// Detail for diagnostics. Not emitted in `Display`/`Debug`.
        message: String,
    },
}

impl OAuthProtocolError {
    /// Map this error to the engine-wide 400/401/502/500 table.
    pub const fn class(&self) -> OAuthErrorClass {
        match self {
            Self::InvalidRequestShape { .. }
            | Self::MalformedTokenResponse { .. }
            | Self::MalformedProviderResponse { .. }
            | Self::ProviderReportedError { .. } => OAuthErrorClass::ClientError,
            Self::IdentityVerificationFailed { .. } => OAuthErrorClass::IdentityError,
            Self::UpstreamUnavailable { .. } => OAuthErrorClass::UpstreamError,
            Self::ProviderConfiguration { .. } => OAuthErrorClass::ServerError,
        }
    }

    /// Provider for this error, when known.
    pub const fn provider(&self) -> Option<&'static str> {
        match self {
            Self::InvalidRequestShape { .. } | Self::MalformedTokenResponse { .. } => None,
            Self::IdentityVerificationFailed { provider, .. }
            | Self::MalformedProviderResponse { provider, .. }
            | Self::ProviderReportedError { provider, .. }
            | Self::UpstreamUnavailable { provider, .. }
            | Self::ProviderConfiguration { provider, .. } => Some(provider),
        }
    }

    /// Build a secret-safe tracing context for a failed OAuth flow.
    pub fn trace_context<'a>(
        &self,
        grant: &'static str,
        ceremony_kind: &'static str,
        correlation_id: &'a str,
    ) -> OAuthErrorTraceContext<'a> {
        OAuthErrorTraceContext {
            class: self.class(),
            provider: self.provider(),
            grant,
            ceremony_kind,
            correlation_id,
        }
    }
}

impl fmt::Display for OAuthProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequestShape { .. } => f.write_str("invalid OAuth request shape"),
            Self::MalformedTokenResponse { .. } => f.write_str("malformed OAuth token response"),
            Self::MalformedProviderResponse { .. } => {
                f.write_str("malformed OAuth provider response")
            }
            Self::ProviderReportedError { .. } => f.write_str("provider reported OAuth error"),
            Self::IdentityVerificationFailed { .. } => {
                f.write_str("provider identity verification failed")
            }
            Self::UpstreamUnavailable { .. } => f.write_str("upstream OAuth provider unavailable"),
            Self::ProviderConfiguration { .. } => f.write_str("provider configuration error"),
        }
    }
}

impl fmt::Debug for OAuthProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequestShape { field, .. } => f
                .debug_struct("InvalidRequestShape")
                .field("field", field)
                .field("class", &self.class())
                .finish(),
            Self::MalformedTokenResponse { .. } => f
                .debug_struct("MalformedTokenResponse")
                .field("class", &self.class())
                .finish(),
            Self::MalformedProviderResponse { provider, .. } => f
                .debug_struct("MalformedProviderResponse")
                .field("provider", provider)
                .field("class", &self.class())
                .finish(),
            Self::ProviderReportedError { provider, code, .. } => f
                .debug_struct("ProviderReportedError")
                .field("provider", provider)
                .field("class", &self.class())
                .field("code", code)
                .finish(),
            Self::IdentityVerificationFailed { provider, .. } => f
                .debug_struct("IdentityVerificationFailed")
                .field("provider", provider)
                .field("class", &self.class())
                .finish(),
            Self::UpstreamUnavailable {
                provider,
                retry_after_seconds,
                ..
            } => f
                .debug_struct("UpstreamUnavailable")
                .field("provider", provider)
                .field("class", &self.class())
                .field("retry_after_seconds", retry_after_seconds)
                .finish(),
            Self::ProviderConfiguration { provider, .. } => f
                .debug_struct("ProviderConfiguration")
                .field("provider", provider)
                .field("class", &self.class())
                .finish(),
        }
    }
}

impl std::error::Error for OAuthProtocolError {}
