//! Closed low-cardinality telemetry for asynchronous delivery pressure.

use std::fmt;

const COUNTER_COUNT: usize = 6;

/// Finite delivery-pressure counter vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum AsyncTelemetryCounter {
    /// A new bounded queue position was admitted.
    Queued = 0,
    /// Replaceable work was absorbed by the current tail through replacement or retention.
    Coalesced = 1,
    /// Exact continuity became uncertain.
    Degraded = 2,
    /// A delivery scope rejected work with a terminal typed code.
    Closed = 3,
    /// Internal bounded validation rejected malformed framework input.
    Rejected = 4,
    /// Owner retirement released retained work.
    Cleanup = 5,
}

impl AsyncTelemetryCounter {
    /// Complete finite label vocabulary.
    pub const ALL: &[Self] = &[
        Self::Queued,
        Self::Coalesced,
        Self::Degraded,
        Self::Closed,
        Self::Rejected,
        Self::Cleanup,
    ];

    /// Returns the stable machine-readable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Coalesced => "coalesced",
            Self::Degraded => "degraded",
            Self::Closed => "closed",
            Self::Rejected => "rejected",
            Self::Cleanup => "cleanup",
        }
    }
}

#[derive(Default)]
pub(crate) struct AsyncTelemetry {
    counters: [u64; COUNTER_COUNT],
}

impl AsyncTelemetry {
    pub(crate) fn increment(&mut self, counter: AsyncTelemetryCounter) {
        let value = &mut self.counters[counter as usize];
        *value = value.saturating_add(1);
    }

    pub(crate) const fn snapshot(&self) -> AsyncTelemetrySnapshot {
        AsyncTelemetrySnapshot {
            counters: self.counters,
        }
    }
}

/// Fixed-size redaction-safe copy of current pressure counters.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AsyncTelemetrySnapshot {
    counters: [u64; COUNTER_COUNT],
}

impl AsyncTelemetrySnapshot {
    /// Returns one counter selected from the closed vocabulary.
    #[must_use]
    pub const fn count(self, counter: AsyncTelemetryCounter) -> u64 {
        self.counters[counter as usize]
    }
}

impl fmt::Debug for AsyncTelemetrySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = formatter.debug_map();
        for counter in AsyncTelemetryCounter::ALL {
            map.entry(&counter.as_str(), &self.counters[*counter as usize]);
        }
        map.finish()
    }
}
