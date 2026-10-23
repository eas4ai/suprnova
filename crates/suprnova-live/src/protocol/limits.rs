//! Independent bounds for Live control envelopes and their nested payload classes.

use crate::limits::{HARD_MAX_COLLECTION_ITEMS, HARD_MAX_INPUT_BYTES, InputLimits};

use super::{ProtocolError, ProtocolErrorKind};

const MAX_NESTED_BYTES: usize = HARD_MAX_INPUT_BYTES;
const MAX_COLLECTION_ITEMS: usize = HARD_MAX_COLLECTION_ITEMS;

/// Ceiling on one redirect or history URL: browsers refuse a URL past 2 MiB,
/// so a longer one could never be followed.
pub const MAX_REDIRECT_BYTES: usize = 2 * 1024 * 1024;
/// The redirect URL bound until [`ProtocolLimits::with_max_redirect_bytes`]
/// sets the configured one (`LIVE_MAX_REDIRECT_BYTES` in the framework).
pub const DEFAULT_REDIRECT_BYTES: usize = 64 * 1024;

/// Raw protocol policy values validated by [`ProtocolLimits`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolLimitConfig {
    /// Whole-envelope canonical parser limits.
    pub input: InputLimits,
    /// Maximum canonical bytes for one embedded signed snapshot.
    pub max_snapshot_bytes: usize,
    /// Maximum UTF-8 bytes for one rendered HTML payload.
    pub max_html_bytes: usize,
    /// Maximum proposed model fields.
    pub max_model_proposals: usize,
    /// Maximum ordered operations.
    pub max_operations: usize,
    /// Maximum arguments on one action operation.
    pub max_arguments: usize,
    /// Maximum validation object entries.
    pub max_validation_entries: usize,
    /// Maximum declared events.
    pub max_events: usize,
    /// Maximum registered effects.
    pub max_effects: usize,
    /// Maximum namespaced extension entries.
    pub max_extensions: usize,
}

/// Validated protocol parsing and nested-payload limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolLimits(ProtocolLimitConfig, usize);

impl ProtocolLimits {
    /// Validates non-zero nested bounds against hard and whole-input ceilings.
    pub fn new(config: ProtocolLimitConfig) -> Result<Self, ProtocolError> {
        let byte_limit_valid = config.max_snapshot_bytes > 0
            && config.max_snapshot_bytes <= config.input.max_bytes()
            && config.max_snapshot_bytes <= MAX_NESTED_BYTES
            && config.max_html_bytes > 0
            && config.max_html_bytes <= config.input.max_bytes()
            && config.max_html_bytes <= config.input.max_string_bytes()
            && config.max_html_bytes <= MAX_NESTED_BYTES;
        let counts = [
            config.max_model_proposals,
            config.max_operations,
            config.max_arguments,
            config.max_validation_entries,
            config.max_events,
            config.max_effects,
            config.max_extensions,
        ];
        if !byte_limit_valid
            || counts
                .iter()
                .any(|count| *count == 0 || *count > MAX_COLLECTION_ITEMS)
        {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
        }
        Ok(Self(config, DEFAULT_REDIRECT_BYTES))
    }

    /// Returns a copy that accepts redirect and history URLs up to `limit`
    /// bytes, from 1 to [`MAX_REDIRECT_BYTES`] and within one whole input.
    pub fn with_max_redirect_bytes(mut self, limit: usize) -> Result<Self, ProtocolError> {
        if limit == 0 || limit > MAX_REDIRECT_BYTES || limit > self.0.input.max_bytes() {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
        }
        self.1 = limit;
        Ok(self)
    }

    /// Returns a copy with a smaller or equal positive operation limit.
    pub fn with_max_operations(mut self, limit: usize) -> Result<Self, ProtocolError> {
        if limit == 0 || limit > self.0.max_operations {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
        }
        self.0.max_operations = limit;
        Ok(self)
    }

    /// Returns a copy with a smaller or equal positive snapshot-byte limit.
    pub fn with_max_snapshot_bytes(mut self, limit: usize) -> Result<Self, ProtocolError> {
        if limit == 0 || limit > self.0.max_snapshot_bytes {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
        }
        self.0.max_snapshot_bytes = limit;
        Ok(self)
    }

    pub(crate) const fn input(&self) -> &InputLimits {
        &self.0.input
    }

    pub(crate) const fn max_snapshot_bytes(&self) -> usize {
        self.0.max_snapshot_bytes
    }

    pub(crate) const fn max_html_bytes(&self) -> usize {
        self.0.max_html_bytes
    }

    pub(crate) const fn max_model_proposals(&self) -> usize {
        self.0.max_model_proposals
    }

    pub(crate) const fn max_operations(&self) -> usize {
        self.0.max_operations
    }

    pub(crate) const fn max_arguments(&self) -> usize {
        self.0.max_arguments
    }

    pub(crate) const fn max_validation_entries(&self) -> usize {
        self.0.max_validation_entries
    }

    pub(crate) const fn max_events(&self) -> usize {
        self.0.max_events
    }

    pub(crate) const fn max_effects(&self) -> usize {
        self.0.max_effects
    }

    pub(crate) const fn max_extensions(&self) -> usize {
        self.0.max_extensions
    }

    pub(crate) const fn max_redirect_bytes(&self) -> usize {
        self.1
    }
}
