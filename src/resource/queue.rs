//! FIFO admission with exact retained-byte accounting.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt;

use super::ResourceBounds;

/// Closed low-cardinality outcome vocabulary for resource diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceDiagnostic {
    /// Bounded work was admitted.
    Accepted,
    /// Accounted work or a permit was released.
    Released,
    /// The queue item ceiling rejected admission.
    ItemsExceeded,
    /// The queue byte ceiling or byte arithmetic rejected admission.
    BytesExceeded,
    /// The active permit ceiling rejected acquisition.
    PermitsExceeded,
    /// One-way cancellation was requested.
    Canceled,
    /// The owning lifecycle was retired.
    Retired,
}

impl ResourceDiagnostic {
    /// Complete bounded diagnostic vocabulary.
    pub const ALL: &[Self] = &[
        Self::Accepted,
        Self::Released,
        Self::ItemsExceeded,
        Self::BytesExceeded,
        Self::PermitsExceeded,
        Self::Canceled,
        Self::Retired,
    ];

    /// Returns the stable machine-readable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Released => "released",
            Self::ItemsExceeded => "items_exceeded",
            Self::BytesExceeded => "bytes_exceeded",
            Self::PermitsExceeded => "permits_exceeded",
            Self::Canceled => "canceled",
            Self::Retired => "retired",
        }
    }
}

/// Closed rejection from bounded resource admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    /// The configured queue item ceiling was reached.
    ItemsExceeded,
    /// The configured byte ceiling or checked byte arithmetic rejected admission.
    BytesExceeded,
    /// The configured active permit ceiling was reached.
    PermitsExceeded,
    /// The owner no longer accepts work.
    Retired,
}

impl ResourceError {
    /// Returns the corresponding closed diagnostic label.
    #[must_use]
    pub const fn diagnostic(self) -> ResourceDiagnostic {
        match self {
            Self::ItemsExceeded => ResourceDiagnostic::ItemsExceeded,
            Self::BytesExceeded => ResourceDiagnostic::BytesExceeded,
            Self::PermitsExceeded => ResourceDiagnostic::PermitsExceeded,
            Self::Retired => ResourceDiagnostic::Retired,
        }
    }

    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.diagnostic().as_str()
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Error for ResourceError {}

#[derive(Debug)]
pub(super) struct BoundedItem<T> {
    bytes: usize,
    value: T,
}

/// A payload-neutral bounded first-in, first-out queue.
///
/// Callers supply the retained byte reservation for each value. The queue does
/// not inspect payloads and therefore cannot infer or accidentally disclose
/// their contents.
#[derive(Debug)]
pub struct BoundedQueue<T> {
    bounds: ResourceBounds,
    retained_bytes: usize,
    items: VecDeque<BoundedItem<T>>,
    retired: bool,
}

impl<T> BoundedQueue<T> {
    /// Creates an empty active queue with validated bounds.
    #[must_use]
    pub fn new(bounds: ResourceBounds) -> Self {
        Self {
            bounds,
            retained_bytes: 0,
            items: VecDeque::new(),
            retired: false,
        }
    }

    /// Admits one value after reserving its declared retained bytes.
    ///
    /// Failed admission leaves item and byte accounting unchanged. Zero-byte
    /// values are allowed and remain bounded by the item ceiling.
    pub fn try_push(&mut self, bytes: usize, value: T) -> Result<(), ResourceError> {
        self.try_push_preserving(bytes, value)
            .map_err(|(error, _rejected)| error)
    }

    pub(super) fn try_push_preserving(
        &mut self,
        bytes: usize,
        value: T,
    ) -> Result<(), (ResourceError, T)> {
        if self.retired {
            return Err((ResourceError::Retired, value));
        }
        let Some(next) = self.retained_bytes.checked_add(bytes) else {
            return Err((ResourceError::BytesExceeded, value));
        };
        if self.items.len() == self.bounds.max_items() {
            return Err((ResourceError::ItemsExceeded, value));
        }
        if next > self.bounds.max_bytes() {
            return Err((ResourceError::BytesExceeded, value));
        }

        self.retained_bytes = next;
        self.items.push_back(BoundedItem { bytes, value });
        Ok(())
    }

    /// Removes the oldest value and releases its byte reservation exactly once.
    pub fn pop(&mut self) -> Option<T> {
        let item = self.items.pop_front()?;
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(item.bytes)
            .expect("bounded queue byte accounting invariant");
        Some(item.value)
    }

    /// Returns the configured queue bounds.
    #[must_use]
    pub const fn bounds(&self) -> ResourceBounds {
        self.bounds
    }

    /// Returns the number of queued values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the queue contains no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the total bytes currently reserved by queued values.
    #[must_use]
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Returns whether the queue permanently rejects new admission.
    #[must_use]
    pub const fn is_retired(&self) -> bool {
        self.retired
    }

    pub(super) fn retire_and_take(&mut self) -> Option<(Retirement, VecDeque<BoundedItem<T>>)> {
        if self.retired {
            return None;
        }
        self.retired = true;
        let drained_items = self.items.len();
        let drained_bytes = self.retained_bytes;
        self.retained_bytes = 0;
        let items = std::mem::take(&mut self.items);
        Some((
            Retirement {
                canceled: false,
                drained_items,
                drained_bytes,
            },
            items,
        ))
    }
}

/// Metadata-only result of retiring one resource owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Retirement {
    /// Whether this retirement performed the first cancellation transition.
    pub canceled: bool,
    /// Number of queued items released by this retirement.
    pub drained_items: usize,
    /// Number of retained bytes released by this retirement.
    pub drained_bytes: usize,
}

impl Retirement {
    /// Returns the no-op result for an owner that was already retired.
    #[must_use]
    pub const fn already_retired() -> Self {
        Self {
            canceled: false,
            drained_items: 0,
            drained_bytes: 0,
        }
    }
}
