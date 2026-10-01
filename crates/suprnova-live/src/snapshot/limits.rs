//! Validated snapshot-specific limits.

use super::{SnapshotError, SnapshotErrorKind};
use crate::limits::InputLimits;

/// Input, clock, validity, generation, and extension limits for snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotLimits {
    input: InputLimits,
    max_clock_skew_ms: u64,
    max_seed_age_ms: u64,
    max_instance_lifetime_ms: u64,
    max_generations: usize,
    max_extensions: usize,
}

impl SnapshotLimits {
    /// Creates non-zero bounded snapshot policy.
    pub fn new(
        input: InputLimits,
        max_clock_skew_ms: u64,
        max_seed_age_ms: u64,
        max_instance_lifetime_ms: u64,
        max_generations: usize,
        max_extensions: usize,
    ) -> Result<Self, SnapshotError> {
        if max_seed_age_ms == 0
            || max_instance_lifetime_ms == 0
            || max_generations == 0
            || max_extensions == 0
            || max_generations > input.max_entries()
            || max_extensions > input.max_entries()
        {
            return Err(SnapshotError::new(SnapshotErrorKind::InvalidSchema));
        }
        Ok(Self {
            input,
            max_clock_skew_ms,
            max_seed_age_ms,
            max_instance_lifetime_ms,
            max_generations,
            max_extensions,
        })
    }

    /// Returns the bounded canonical input policy.
    #[must_use]
    pub const fn input(&self) -> &InputLimits {
        &self.input
    }

    pub(crate) const fn max_clock_skew_ms(&self) -> u64 {
        self.max_clock_skew_ms
    }

    pub(crate) const fn max_seed_age_ms(&self) -> u64 {
        self.max_seed_age_ms
    }

    pub(crate) const fn max_instance_lifetime_ms(&self) -> u64 {
        self.max_instance_lifetime_ms
    }

    pub(crate) const fn max_generations(&self) -> usize {
        self.max_generations
    }

    pub(crate) const fn max_extensions(&self) -> usize {
        self.max_extensions
    }
}
