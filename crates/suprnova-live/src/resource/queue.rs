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

pub(super) struct BoundedItem<T> {
    bytes: usize,
    value: T,
}

pub(super) enum ReplaceBack<T> {
    Replaced(T),
    Empty(T),
}

/// Lock-scoped decision for admitting a value relative to the current tail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TailAdmission {
    /// Append the value as a distinct FIFO item.
    Append,
    /// Replace the current tail without changing its FIFO position.
    Replace,
    /// Retain the current tail and discard the semantically redundant value.
    Retain,
    /// Reject the value without mutating the queue.
    Reject,
}

/// Result of a successful lock-scoped tail admission decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TailAdmissionOutcome {
    /// The value was appended as a distinct FIFO item.
    Appended,
    /// The value replaced the current tail.
    Replaced,
    /// The existing tail was retained because the value was redundant.
    Retained,
    /// The value was rejected by the caller's semantic decision.
    Rejected,
}

pub(super) enum TailAdmissionPreserving<T> {
    Appended,
    Replaced(T),
    Retained(T),
    Rejected(T),
}

pub(super) struct RemovedItems<T> {
    pub(super) items: VecDeque<BoundedItem<T>>,
    pub(super) count: usize,
    pub(super) bytes: usize,
}

/// A payload-neutral bounded first-in, first-out queue.
///
/// Callers supply the retained byte reservation for each value. The queue does
/// not inspect payloads and therefore cannot infer or accidentally disclose
/// their contents.
pub struct BoundedQueue<T> {
    bounds: ResourceBounds,
    retained_bytes: usize,
    items: VecDeque<BoundedItem<T>>,
    retired: bool,
}

impl<T> fmt::Debug for BoundedQueue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedQueue")
            .field("bounds", &self.bounds)
            .field("items", &self.items.len())
            .field("retained_bytes", &self.retained_bytes)
            .field("retired", &self.retired)
            .finish()
    }
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

    /// Replaces the newest queued value while preserving its FIFO position.
    ///
    /// The replacement reserves its declared bytes atomically. Failed
    /// replacement leaves the existing value and accounting unchanged. An
    /// empty queue returns `Ok(None)` and consumes no reservation.
    pub fn try_replace_back(&mut self, bytes: usize, value: T) -> Result<Option<T>, ResourceError> {
        match self.try_replace_back_preserving(bytes, value) {
            Ok(ReplaceBack::Replaced(previous)) => Ok(Some(previous)),
            Ok(ReplaceBack::Empty(rejected)) => {
                drop(rejected);
                Ok(None)
            }
            Err((error, rejected)) => {
                drop(rejected);
                Err(error)
            }
        }
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

    pub(super) fn try_push_batch_preserving(
        &mut self,
        total_bytes: usize,
        values: Vec<(usize, T)>,
    ) -> Result<(), (ResourceError, Vec<(usize, T)>)> {
        if self.retired {
            return Err((ResourceError::Retired, values));
        }
        let Some(next_items) = self.items.len().checked_add(values.len()) else {
            return Err((ResourceError::ItemsExceeded, values));
        };
        if next_items > self.bounds.max_items() {
            return Err((ResourceError::ItemsExceeded, values));
        }
        let Some(next_bytes) = self.retained_bytes.checked_add(total_bytes) else {
            return Err((ResourceError::BytesExceeded, values));
        };
        if next_bytes > self.bounds.max_bytes() {
            return Err((ResourceError::BytesExceeded, values));
        }

        self.items.extend(
            values
                .into_iter()
                .map(|(bytes, value)| BoundedItem { bytes, value }),
        );
        self.retained_bytes = next_bytes;
        Ok(())
    }

    pub(super) fn back(&self) -> Option<&T> {
        self.items.back().map(|item| &item.value)
    }

    pub(super) fn try_admit_tail_preserving(
        &mut self,
        bytes: usize,
        value: T,
        decision: TailAdmission,
    ) -> Result<TailAdmissionPreserving<T>, (ResourceError, T)> {
        if self.retired {
            return Err((ResourceError::Retired, value));
        }

        match decision {
            TailAdmission::Append => self
                .try_push_preserving(bytes, value)
                .map(|()| TailAdmissionPreserving::Appended),
            TailAdmission::Replace => match self.try_replace_back_preserving(bytes, value) {
                Ok(ReplaceBack::Replaced(previous)) => {
                    Ok(TailAdmissionPreserving::Replaced(previous))
                }
                Ok(ReplaceBack::Empty(rejected)) => Ok(TailAdmissionPreserving::Rejected(rejected)),
                Err(rejected) => Err(rejected),
            },
            TailAdmission::Retain => Ok(TailAdmissionPreserving::Retained(value)),
            TailAdmission::Reject => Ok(TailAdmissionPreserving::Rejected(value)),
        }
    }

    pub(super) fn remove_if_preserving<F>(&mut self, predicate: &mut F) -> RemovedItems<T>
    where
        F: FnMut(&T) -> bool,
    {
        let decisions = self
            .items
            .iter()
            .map(|item| predicate(&item.value))
            .collect::<Vec<_>>();
        let mut kept = VecDeque::with_capacity(self.items.len());
        let mut removed = VecDeque::new();
        let mut removed_bytes = 0usize;
        for (item, remove) in std::mem::take(&mut self.items).into_iter().zip(decisions) {
            if remove {
                removed_bytes = removed_bytes
                    .checked_add(item.bytes)
                    .expect("bounded queue removal byte invariant");
                removed.push_back(item);
            } else {
                kept.push_back(item);
            }
        }
        self.items = kept;
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(removed_bytes)
            .expect("bounded queue removal byte accounting invariant");
        RemovedItems {
            count: removed.len(),
            bytes: removed_bytes,
            items: removed,
        }
    }

    pub(super) fn any<F>(&self, predicate: &mut F) -> bool
    where
        F: FnMut(&T) -> bool,
    {
        self.items.iter().any(|item| predicate(&item.value))
    }

    pub(super) fn pop_batch_preserving<F>(&mut self, classify: &mut F) -> Option<Vec<T>>
    where
        F: FnMut(usize, &T) -> Option<usize>,
    {
        let first = self.items.front()?;
        let count = classify(0, &first.value)?;
        if count == 0 || count > self.items.len() {
            return None;
        }
        if self
            .items
            .iter()
            .take(count)
            .enumerate()
            .skip(1)
            .any(|(index, item)| classify(index, &item.value) != Some(count))
        {
            return None;
        }

        let mut values = Vec::with_capacity(count);
        let mut released_bytes = 0usize;
        for _ in 0..count {
            let item = self
                .items
                .pop_front()
                .expect("validated queue prefix remains present while locked");
            released_bytes = released_bytes
                .checked_add(item.bytes)
                .expect("bounded queue batch-pop byte invariant");
            values.push(item.value);
        }
        self.retained_bytes = self
            .retained_bytes
            .checked_sub(released_bytes)
            .expect("bounded queue batch-pop accounting invariant");
        Some(values)
    }

    pub(super) fn try_replace_back_preserving(
        &mut self,
        bytes: usize,
        value: T,
    ) -> Result<ReplaceBack<T>, (ResourceError, T)> {
        if self.retired {
            return Err((ResourceError::Retired, value));
        }
        let Some(current) = self.items.back() else {
            return Ok(ReplaceBack::Empty(value));
        };
        let retained_without_current = self
            .retained_bytes
            .checked_sub(current.bytes)
            .expect("bounded queue byte accounting invariant");
        let Some(next) = retained_without_current.checked_add(bytes) else {
            return Err((ResourceError::BytesExceeded, value));
        };
        if next > self.bounds.max_bytes() {
            return Err((ResourceError::BytesExceeded, value));
        }

        let current = self
            .items
            .back_mut()
            .expect("bounded queue newest item remains present");
        let previous = std::mem::replace(&mut current.value, value);
        current.bytes = bytes;
        self.retained_bytes = next;
        Ok(ReplaceBack::Replaced(previous))
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
