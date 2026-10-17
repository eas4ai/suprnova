//! Validated hard bounds for retained resource work.

use std::error::Error;
use std::fmt;

/// Hard ceiling for queued items owned by one bounded resource scope.
pub const HARD_MAX_RESOURCE_ITEMS: usize = 65_536;
/// Hard ceiling for accounted bytes owned by one bounded resource scope.
pub const HARD_MAX_RESOURCE_BYTES: usize = 1024 * 1024 * 1024;
/// Hard ceiling for simultaneously active permits in one pool.
pub const HARD_MAX_ACTIVE_PERMITS: usize = 65_536;

/// Validated item and byte ceilings for one bounded queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBounds {
    max_items: usize,
    max_bytes: usize,
}

impl ResourceBounds {
    /// Creates non-zero bounds below the engine hard ceilings.
    pub fn new(max_items: usize, max_bytes: usize) -> Result<Self, ResourceBoundsError> {
        if max_items == 0
            || max_items > HARD_MAX_RESOURCE_ITEMS
            || max_bytes == 0
            || max_bytes > HARD_MAX_RESOURCE_BYTES
        {
            return Err(ResourceBoundsError);
        }

        Ok(Self {
            max_items,
            max_bytes,
        })
    }

    /// Returns the maximum queued item count.
    #[must_use]
    pub const fn max_items(self) -> usize {
        self.max_items
    }

    /// Returns the maximum total accounted queue bytes.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

/// A resource count or byte ceiling was zero or exceeded an engine hard limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBoundsError;

impl fmt::Display for ResourceBoundsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_resource_bounds")
    }
}

impl Error for ResourceBoundsError {}

pub(crate) fn validate_permit_limit(max_active: usize) -> Result<(), ResourceBoundsError> {
    if max_active == 0 || max_active > HARD_MAX_ACTIVE_PERMITS {
        return Err(ResourceBoundsError);
    }
    Ok(())
}
