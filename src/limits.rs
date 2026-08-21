//! Resource limits used before external input can amplify work.

use std::error::Error;
use std::fmt;

/// Hard ceiling for one iteration 001 control or snapshot input.
pub const HARD_MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
/// Hard ceiling for JSON container nesting.
pub const HARD_MAX_DEPTH: usize = 64;
/// Hard ceiling for total array elements plus object members.
pub const HARD_MAX_ENTRIES: usize = 100_000;
/// Hard ceiling for one decoded JSON string.
pub const HARD_MAX_STRING_BYTES: usize = 1024 * 1024;

/// Validated byte, depth, collection, and string limits for an input boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputLimits {
    max_bytes: usize,
    max_depth: usize,
    max_entries: usize,
    max_string_bytes: usize,
}

impl InputLimits {
    /// Creates limits that are non-zero and below the engine hard ceilings.
    pub fn new(
        max_bytes: usize,
        max_depth: usize,
        max_entries: usize,
        max_string_bytes: usize,
    ) -> Result<Self, LimitConfigurationError> {
        let within_ceiling = max_bytes <= HARD_MAX_INPUT_BYTES
            && max_depth <= HARD_MAX_DEPTH
            && max_entries <= HARD_MAX_ENTRIES
            && max_string_bytes <= HARD_MAX_STRING_BYTES;
        let non_zero = max_bytes > 0 && max_depth > 0 && max_entries > 0 && max_string_bytes > 0;

        if !within_ceiling || !non_zero {
            return Err(LimitConfigurationError);
        }

        Ok(Self {
            max_bytes,
            max_depth,
            max_entries,
            max_string_bytes,
        })
    }

    /// Maximum encoded input bytes accepted before parsing.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    /// Maximum nested array/object container count.
    #[must_use]
    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    /// Maximum total array elements plus object members.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Maximum decoded UTF-8 bytes in one string or object key.
    #[must_use]
    pub const fn max_string_bytes(self) -> usize {
        self.max_string_bytes
    }
}

impl Default for InputLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
            max_depth: 32,
            max_entries: 2_048,
            max_string_bytes: 16 * 1024,
        }
    }
}

/// A configured input limit was zero or exceeded an engine hard ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitConfigurationError;

impl fmt::Display for LimitConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_limit_configuration")
    }
}

impl Error for LimitConfigurationError {}
