//! Injectable wall-clock boundary for expiry and lease decisions.

use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::identity::UnixMillis;

/// Closed failure reason for obtaining a usable Unix timestamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockErrorKind {
    /// The system clock reported a time before the Unix epoch.
    BeforeUnixEpoch,
    /// Milliseconds since the Unix epoch exceeded the supported integer range.
    TimestampOverflow,
}

impl ClockErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforeUnixEpoch => "before_unix_epoch",
            Self::TimestampOverflow => "timestamp_overflow",
        }
    }
}

/// Redacted clock-provider failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ClockError {
    kind: ClockErrorKind,
}

impl ClockError {
    const fn new(kind: ClockErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure reason.
    #[must_use]
    pub const fn kind(self) -> ClockErrorKind {
        self.kind
    }
}

impl fmt::Display for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ClockError {}

/// Supplies current Unix milliseconds to time-sensitive providers.
pub trait Clock: Send + Sync {
    /// Returns the current wall-clock time or a closed provider error.
    fn now(&self) -> Result<UnixMillis, ClockError>;
}

/// Production wall clock backed by [`SystemTime`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ClockError::new(ClockErrorKind::BeforeUnixEpoch))?;
        let milliseconds = u64::try_from(duration.as_millis())
            .map_err(|_| ClockError::new(ClockErrorKind::TimestampOverflow))?;
        Ok(UnixMillis::new(milliseconds))
    }
}
