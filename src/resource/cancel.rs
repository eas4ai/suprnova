//! One-way cross-thread cancellation state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cloneable one-way cancellation flag for executor-owned work.
///
/// Cancellation is advisory: consumers decide where their bounded work can
/// safely observe the flag and stop. Calling [`Self::cancel`] never spawns or
/// dispatches work.
#[derive(Clone, Debug, Default)]
pub struct CancellationFlag {
    canceled: Arc<AtomicBool>,
}

impl CancellationFlag {
    /// Creates an active flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the flag canceled, returning `true` only for the first transition.
    pub fn cancel(&self) -> bool {
        !self.canceled.swap(true, Ordering::AcqRel)
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}
