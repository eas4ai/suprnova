//! Closed, redacted telemetry for upload cleanup reconciliation.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::identity::UnixMillis;

/// Bounded age classification for one cleanup claim.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UploadAgeBucket {
    /// Less than one minute old.
    UnderMinute,
    /// At least one minute but less than one hour old.
    UnderHour,
    /// At least one hour but less than one day old.
    UnderDay,
    /// At least one day old.
    DayOrOlder,
}

impl UploadAgeBucket {
    /// Every value in stable telemetry order.
    pub const ALL: [Self; 4] = [
        Self::UnderMinute,
        Self::UnderHour,
        Self::UnderDay,
        Self::DayOrOlder,
    ];

    pub(crate) fn classify(created_at: UnixMillis, observed_at: UnixMillis) -> Self {
        match observed_at.get().saturating_sub(created_at.get()) {
            0..=59_999 => Self::UnderMinute,
            60_000..=3_599_999 => Self::UnderHour,
            3_600_000..=86_399_999 => Self::UnderDay,
            _ => Self::DayOrOlder,
        }
    }
}

/// Bounded retained-volume classification for one cleanup claim.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UploadVolumeBucket {
    /// No retained file bytes were recorded.
    Empty,
    /// At most 64 KiB was retained.
    UpTo64KiB,
    /// At most 1 MiB was retained.
    UpTo1MiB,
    /// At most 64 MiB was retained.
    UpTo64MiB,
    /// More than 64 MiB was retained.
    Over64MiB,
}

impl UploadVolumeBucket {
    /// Every value in stable telemetry order.
    pub const ALL: [Self; 5] = [
        Self::Empty,
        Self::UpTo64KiB,
        Self::UpTo1MiB,
        Self::UpTo64MiB,
        Self::Over64MiB,
    ];

    pub(crate) const fn classify(bytes: u64) -> Self {
        match bytes {
            0 => Self::Empty,
            1..=65_536 => Self::UpTo64KiB,
            65_537..=1_048_576 => Self::UpTo1MiB,
            1_048_577..=67_108_864 => Self::UpTo64MiB,
            _ => Self::Over64MiB,
        }
    }
}

/// Closed cleanup result classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CleanupOutcome {
    /// Provider bytes and validation evidence were reclaimed.
    Reclaimed,
    /// A bounded retry was scheduled after a failed reclamation.
    RetryScheduled,
    /// A claim was returned without counting a failed attempt.
    Deferred,
    /// Fenced completion authority was no longer current.
    LeaseLost,
}

impl CleanupOutcome {
    /// Every value in stable telemetry order.
    pub const ALL: [Self; 4] = [
        Self::Reclaimed,
        Self::RetryScheduled,
        Self::Deferred,
        Self::LeaseLost,
    ];
}

/// Bounded retry-count classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RetryBucket {
    /// No failed reclamation has occurred.
    None,
    /// This is the first failed attempt.
    One,
    /// Between two and four failed attempts have occurred.
    TwoToFour,
    /// Between five and eight failed attempts have occurred.
    FiveToEight,
    /// Nine or more failed attempts have occurred.
    NineOrMore,
}

impl RetryBucket {
    /// Every value in stable telemetry order.
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::One,
        Self::TwoToFour,
        Self::FiveToEight,
        Self::NineOrMore,
    ];

    pub(crate) const fn classify(attempt: u32) -> Self {
        match attempt {
            0 => Self::None,
            1 => Self::One,
            2..=4 => Self::TwoToFour,
            5..=8 => Self::FiveToEight,
            _ => Self::NineOrMore,
        }
    }
}

/// Identifier-free metric value emitted after one cleanup completion attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CleanupMetrics {
    age_bucket: UploadAgeBucket,
    volume_bucket: UploadVolumeBucket,
    outcome: CleanupOutcome,
    retry_bucket: RetryBucket,
    orphaned: bool,
}

impl CleanupMetrics {
    pub(crate) const fn new(
        age_bucket: UploadAgeBucket,
        volume_bucket: UploadVolumeBucket,
        outcome: CleanupOutcome,
        retry_bucket: RetryBucket,
        orphaned: bool,
    ) -> Self {
        Self {
            age_bucket,
            volume_bucket,
            outcome,
            retry_bucket,
            orphaned,
        }
    }

    /// Returns the bounded age classification.
    #[must_use]
    pub const fn age_bucket(self) -> UploadAgeBucket {
        self.age_bucket
    }

    /// Returns the bounded retained-volume classification.
    #[must_use]
    pub const fn volume_bucket(self) -> UploadVolumeBucket {
        self.volume_bucket
    }

    /// Returns the closed cleanup outcome.
    #[must_use]
    pub const fn outcome(self) -> CleanupOutcome {
        self.outcome
    }

    /// Returns the bounded failed-attempt classification.
    #[must_use]
    pub const fn retry_bucket(self) -> RetryBucket {
        self.retry_bucket
    }

    /// Returns whether the authority record requires orphan reconciliation.
    #[must_use]
    pub const fn orphaned(self) -> bool {
        self.orphaned
    }
}

/// Non-authoritative observer for closed cleanup metrics.
pub trait CleanupMetricSink: Send + Sync {
    /// Records one identifier-free cleanup result.
    fn record(&self, metrics: CleanupMetrics);
}

pub(crate) fn record_metrics(sink: Option<&dyn CleanupMetricSink>, metrics: CleanupMetrics) {
    if let Some(sink) = sink {
        let _ = catch_unwind(AssertUnwindSafe(|| sink.record(metrics)));
    }
}
