//! Stable lifecycle phases and redacted executor failures.

use std::error::Error;
use std::fmt;

/// Closed phase ordering used by lifecycle diagnostics and tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecyclePhase {
    /// Fresh component construction and mount initialization.
    Mount,
    /// Verified state reconstruction.
    Hydrate,
    /// Mutable pre-render lifecycle hook.
    Rendering,
    /// Immutable island rendering.
    Render,
    /// Mutable post-render lifecycle hook.
    Rendered,
    /// Mutable pre-dehydration lifecycle hook.
    Dehydrating,
    /// Immutable state and memo dehydration.
    Dehydrate,
    /// Exactly-once request resource cleanup.
    Teardown,
}

/// Closed executor failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleErrorKind {
    /// Application or generated component code returned an error.
    ComponentFailure,
    /// Component code panicked while its future or synchronous hook was executing.
    Panicked,
    /// The created object did not implement the registered component contract.
    ContractMismatch,
    /// The registry descriptor has no executable generated hooks.
    HooksUnavailable,
}

/// Redacted phase-specific lifecycle failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LifecycleError {
    kind: LifecycleErrorKind,
    phase: LifecyclePhase,
    teardown_failed: bool,
}

impl LifecycleError {
    pub(crate) const fn new(kind: LifecycleErrorKind, phase: LifecyclePhase) -> Self {
        Self {
            kind,
            phase,
            teardown_failed: false,
        }
    }

    pub(crate) const fn with_teardown_failure(mut self) -> Self {
        self.teardown_failed = true;
        self
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> LifecycleErrorKind {
        self.kind
    }

    /// Returns the phase that produced the primary failure.
    #[must_use]
    pub const fn phase(self) -> LifecyclePhase {
        self.phase
    }

    /// Returns whether cleanup also failed after the primary failure.
    #[must_use]
    pub const fn teardown_failed(self) -> bool {
        self.teardown_failed
    }
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "component_lifecycle_failure:{:?}:{:?}",
            self.phase, self.kind
        )
    }
}

impl fmt::Debug for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LifecycleError {}
