//! Cross-thread permit and queue ownership with exact retirement.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::bounds::validate_permit_limit;
use super::queue::{RemovedItems, ReplaceBack, TailAdmissionPreserving};
use super::{
    BoundedQueue, CancellationFlag, ResourceBounds, ResourceBoundsError, ResourceError, Retirement,
    TailAdmission, TailAdmissionOutcome,
};

#[derive(Debug)]
struct PermitPoolInner {
    max_active: usize,
    active: AtomicUsize,
}

/// Cloneable bounded source of active-work permits.
#[derive(Clone, Debug)]
pub struct PermitPool {
    inner: Arc<PermitPoolInner>,
}

impl PermitPool {
    /// Creates an empty permit pool with a non-zero hard-bounded capacity.
    pub fn new(max_active: usize) -> Result<Self, ResourceBoundsError> {
        validate_permit_limit(max_active)?;
        Ok(Self {
            inner: Arc::new(PermitPoolInner {
                max_active,
                active: AtomicUsize::new(0),
            }),
        })
    }

    /// Attempts to acquire one permit without waiting or spawning work.
    pub fn try_acquire(&self) -> Result<Permit, ResourceError> {
        let mut current = self.inner.active.load(Ordering::Acquire);
        loop {
            if current >= self.inner.max_active {
                return Err(ResourceError::PermitsExceeded);
            }
            match self.inner.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Permit {
                        pool: Arc::clone(&self.inner),
                        released: false,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// Returns the configured maximum active permit count.
    #[must_use]
    pub fn max_active(&self) -> usize {
        self.inner.max_active
    }

    /// Returns the number of permits currently held.
    #[must_use]
    pub fn active(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    /// Returns the number of permits immediately available.
    #[must_use]
    pub fn available(&self) -> usize {
        self.inner.max_active - self.active()
    }
}

/// One non-cloneable active-work permit released on consumption or drop.
pub struct Permit {
    pool: Arc<PermitPoolInner>,
    released: bool,
}

impl Permit {
    /// Releases this permit before the end of its lexical scope.
    pub fn release(mut self) {
        self.release_once();
    }

    fn release_once(&mut self) {
        if self.released {
            return;
        }
        let previous = self.pool.active.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "permit accounting underflow");
        self.released = true;
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.release_once();
    }
}

impl fmt::Debug for Permit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Permit")
            .field("released", &self.released)
            .finish_non_exhaustive()
    }
}

/// Thread-safe handle to one owner's bounded FIFO queue.
pub struct ResourceQueue<T> {
    state: Arc<Mutex<BoundedQueue<T>>>,
}

impl<T> Clone for ResourceQueue<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<T> ResourceQueue<T> {
    fn new(bounds: ResourceBounds) -> Self {
        Self {
            state: Arc::new(Mutex::new(BoundedQueue::new(bounds))),
        }
    }

    /// Admits a value after atomically reserving its declared bytes.
    pub fn try_push(&self, bytes: usize, value: T) -> Result<(), ResourceError> {
        let admission = self.lock().try_push_preserving(bytes, value);
        match admission {
            Ok(()) => Ok(()),
            Err((error, rejected)) => {
                // Drop caller-owned payloads after the accounting guard so even
                // a hostile destructor cannot poison the queue mutex.
                drop(rejected);
                Err(error)
            }
        }
    }

    /// Atomically admits an ordered batch or rejects the entire batch.
    ///
    /// Aggregate byte arithmetic is validated before locking. Queue item and
    /// byte ceilings are then rechecked with the current state under one lock,
    /// and the entire batch is appended before that lock is released. Rejected
    /// values are dropped only after the lock is released.
    pub fn try_push_batch(&self, values: Vec<(usize, T)>) -> Result<(), ResourceError> {
        let Some(total_bytes) = values
            .iter()
            .try_fold(0usize, |total, (bytes, _)| total.checked_add(*bytes))
        else {
            drop(values);
            return Err(ResourceError::BytesExceeded);
        };

        let admission = {
            let mut queue = self.lock();
            queue.try_push_batch_preserving(total_bytes, values)
        };
        match admission {
            Ok(()) => Ok(()),
            Err((error, rejected)) => {
                drop(rejected);
                Err(error)
            }
        }
    }

    /// Decides and performs append, tail replacement, or rejection under one
    /// queue lock.
    ///
    /// The decision callback may inspect only the current tail and runs before
    /// mutation. Replaced and rejected values, along with the callback itself,
    /// are dropped after the queue lock is released.
    pub fn try_admit_tail_with<F>(
        &self,
        bytes: usize,
        value: T,
        mut decide: F,
    ) -> Result<TailAdmissionOutcome, ResourceError>
    where
        F: FnMut(Option<&T>) -> TailAdmission,
    {
        let mut pending = Some(value);
        let admission = {
            let mut queue = self.lock();
            let decision = decide(queue.back());
            queue.try_admit_tail_preserving(
                bytes,
                pending
                    .take()
                    .expect("tail admission retains its value through classification"),
                decision,
            )
        };
        drop(decide);
        match admission {
            Ok(TailAdmissionPreserving::Appended) => Ok(TailAdmissionOutcome::Appended),
            Ok(TailAdmissionPreserving::Replaced(previous)) => {
                drop(previous);
                Ok(TailAdmissionOutcome::Replaced)
            }
            Ok(TailAdmissionPreserving::Rejected(rejected)) => {
                drop(rejected);
                Ok(TailAdmissionOutcome::Rejected)
            }
            Err((error, rejected)) => {
                drop(rejected);
                Err(error)
            }
        }
    }

    /// Removes every matching value and releases its reservation atomically.
    ///
    /// The predicate observes one lock-scoped snapshot. Removed values and the
    /// predicate itself are dropped only after the queue lock is released.
    pub fn remove_if<F>(&self, mut predicate: F) -> (usize, usize)
    where
        F: FnMut(&T) -> bool,
    {
        let RemovedItems {
            items,
            count,
            bytes,
        } = {
            let mut queue = self.lock();
            queue.remove_if_preserving(&mut predicate)
        };
        drop(predicate);
        drop(items);
        (count, bytes)
    }

    /// Returns whether one lock-scoped queue snapshot contains a match.
    pub fn any<F>(&self, mut predicate: F) -> bool
    where
        F: FnMut(&T) -> bool,
    {
        let matched = {
            let queue = self.lock();
            queue.any(&mut predicate)
        };
        drop(predicate);
        matched
    }

    /// Replaces the newest queued value without changing its FIFO position.
    ///
    /// Returns `true` when a queued value was replaced and `false` when the
    /// queue was empty. Replaced and rejected payloads are dropped only after
    /// the queue accounting lock is released.
    pub fn try_replace_back(&self, bytes: usize, value: T) -> Result<bool, ResourceError> {
        let replacement = self.lock().try_replace_back_preserving(bytes, value);
        match replacement {
            Ok(ReplaceBack::Replaced(previous)) => {
                drop(previous);
                Ok(true)
            }
            Ok(ReplaceBack::Empty(rejected)) => {
                drop(rejected);
                Ok(false)
            }
            Err((error, rejected)) => {
                drop(rejected);
                Err(error)
            }
        }
    }

    /// Removes the oldest value and releases its byte reservation.
    pub fn pop(&self) -> Option<T> {
        self.lock().pop()
    }

    /// Returns the number of queued values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Returns whether the queue contains no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Returns the total bytes currently reserved by queued values.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.lock().retained_bytes()
    }

    /// Returns the immutable queue item and retained-byte ceilings.
    #[must_use]
    pub fn bounds(&self) -> ResourceBounds {
        self.lock().bounds()
    }

    /// Returns whether the owning lifecycle has retired this queue.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.lock().is_retired()
    }

    fn lock(&self) -> MutexGuard<'_, BoundedQueue<T>> {
        // The guard is never exposed and no payload callback or payload drop runs
        // while it is held. Every mutation restores the queue invariant before
        // returning, so recovering a poisoned guard cannot expose partial user
        // work or double-release accounting.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<T> fmt::Debug for ResourceQueue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let queue = self.lock();
        formatter
            .debug_struct("ResourceQueue")
            .field("bounds", &queue.bounds())
            .field("items", &queue.len())
            .field("retained_bytes", &queue.retained_bytes())
            .field("retired", &queue.is_retired())
            .finish()
    }
}

/// Lifecycle owner for a bounded queue and one-way cancellation flag.
///
/// Retirement is serialized against queue admission. Values drained during
/// retirement are dropped only after the internal lock is released, so a
/// payload destructor panic cannot poison lifecycle accounting.
pub struct ResourceOwner<T> {
    queue: ResourceQueue<T>,
    cancellation: CancellationFlag,
}

impl<T> ResourceOwner<T> {
    /// Creates one active owner with an empty bounded queue.
    #[must_use]
    pub fn new(bounds: ResourceBounds) -> Self {
        Self {
            queue: ResourceQueue::new(bounds),
            cancellation: CancellationFlag::new(),
        }
    }

    /// Returns the synchronized queue owned by this lifecycle.
    #[must_use]
    pub const fn queue(&self) -> &ResourceQueue<T> {
        &self.queue
    }

    /// Returns a cloneable observer for this owner's cancellation state.
    #[must_use]
    pub fn cancellation(&self) -> CancellationFlag {
        self.cancellation.clone()
    }

    /// Cancels the owner and drains its queue exactly once.
    ///
    /// Concurrent queue admission is either admitted before retirement and
    /// drained by this call, or rejected after retirement with
    /// [`ResourceError::Retired`].
    pub fn retire(&self) -> Retirement {
        let (retirement, drained) = {
            let mut queue = self.queue.lock();
            let Some((mut retirement, drained)) = queue.retire_and_take() else {
                return Retirement::already_retired();
            };
            retirement.canceled = self.cancellation.cancel();
            (retirement, drained)
        };
        drop(drained);
        retirement
    }
}

impl<T> Drop for ResourceOwner<T> {
    fn drop(&mut self) {
        self.retire();
    }
}

impl<T> fmt::Debug for ResourceOwner<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceOwner")
            .field("queue", &self.queue)
            .field("canceled", &self.cancellation.is_canceled())
            .finish()
    }
}
