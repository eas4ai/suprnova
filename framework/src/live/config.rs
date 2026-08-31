//! Validated application configuration for Suprnova Live.

use std::error::Error;
use std::fmt;

const HARD_MAX_CONTROL_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_CONTROL_BYTES: usize = 1024 * 1024;

/// Validated byte limits applied to one Live control request and response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfig {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

impl LiveConfig {
    /// Starts a builder with conservative one-megabyte control-envelope limits.
    #[must_use]
    pub const fn builder() -> LiveConfigBuilder {
        LiveConfigBuilder::new()
    }

    /// Returns conservative defaults suitable for an ordinary application.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: DEFAULT_CONTROL_BYTES,
        }
    }

    /// Returns the maximum accepted bytes for one complete Live request body.
    #[must_use]
    pub const fn max_request_bytes(self) -> usize {
        self.max_request_bytes
    }

    /// Returns the maximum emitted bytes for one complete Live response body.
    #[must_use]
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// Startup-only builder for [`LiveConfig`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfigBuilder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

impl LiveConfigBuilder {
    /// Creates a builder with [`LiveConfig::standard`] byte limits.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: DEFAULT_CONTROL_BYTES,
        }
    }

    /// Sets the whole-request body ceiling.
    #[must_use]
    pub const fn max_request_bytes(mut self, max: usize) -> Self {
        self.max_request_bytes = max;
        self
    }

    /// Sets the complete encoded response-body ceiling.
    #[must_use]
    pub const fn max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// Validates the configured limits and creates immutable Live configuration.
    pub fn build(self) -> Result<LiveConfig, LiveConfigError> {
        let valid = (1..=HARD_MAX_CONTROL_BYTES).contains(&self.max_request_bytes)
            && (1..=HARD_MAX_CONTROL_BYTES).contains(&self.max_response_bytes)
            && self.max_response_bytes <= self.max_request_bytes;
        if !valid {
            return Err(LiveConfigError::new(LiveConfigErrorKind::InvalidByteLimits));
        }
        Ok(LiveConfig {
            max_request_bytes: self.max_request_bytes,
            max_response_bytes: self.max_response_bytes,
        })
    }
}

impl Default for LiveConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Closed reason Live startup configuration was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LiveConfigErrorKind {
    /// A request or response ceiling was zero, above the hard ceiling, or inconsistent.
    InvalidByteLimits,
}

impl LiveConfigErrorKind {
    /// Returns the stable machine-readable failure value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidByteLimits => "invalid_live_byte_limits",
        }
    }
}

/// Redacted Live configuration failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LiveConfigError {
    kind: LiveConfigErrorKind,
}

impl LiveConfigError {
    const fn new(kind: LiveConfigErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed configuration failure category.
    #[must_use]
    pub const fn kind(self) -> LiveConfigErrorKind {
        self.kind
    }
}

impl fmt::Display for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LiveConfigError {}
