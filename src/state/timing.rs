//! Closed server/checker metadata for model synchronization timing.

use std::error::Error;
use std::fmt;

const MAX_DEBOUNCE_MILLIS: u32 = 60_000;

/// Closed model synchronization timing declaration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BindingTiming {
    /// Synchronize on every supported control input event.
    Immediate,
    /// Synchronize on the control's change event.
    Change,
    /// Synchronize when focus leaves the control.
    Blur,
    /// Synchronize only with the enclosing submit/action request.
    #[default]
    Submit,
    /// Synchronize after a bounded quiet period.
    Debounce(u32),
}

impl BindingTiming {
    /// Creates a nonzero debounce no longer than sixty seconds.
    pub fn debounce(milliseconds: u32) -> Result<Self, TimingError> {
        if milliseconds == 0 || milliseconds > MAX_DEBOUNCE_MILLIS {
            return Err(TimingError {
                kind: TimingErrorKind::InvalidDebounce,
            });
        }
        Ok(Self::Debounce(milliseconds))
    }

    /// Returns the configured debounce duration, if this is a debounce policy.
    #[must_use]
    pub const fn debounce_millis(self) -> Option<u32> {
        match self {
            Self::Debounce(milliseconds) => Some(milliseconds),
            _ => None,
        }
    }

    pub(crate) const fn is_valid(self) -> bool {
        match self {
            Self::Debounce(milliseconds) => milliseconds > 0 && milliseconds <= MAX_DEBOUNCE_MILLIS,
            _ => true,
        }
    }
}

/// Closed timing declaration failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingErrorKind {
    /// A debounce duration was zero or exceeded the hard ceiling.
    InvalidDebounce,
}

/// Redacted timing metadata error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimingError {
    kind: TimingErrorKind,
}

impl TimingError {
    /// Returns the closed timing failure.
    #[must_use]
    pub const fn kind(self) -> TimingErrorKind {
        self.kind
    }
}

impl fmt::Display for TimingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_binding_timing")
    }
}

impl Error for TimingError {}
