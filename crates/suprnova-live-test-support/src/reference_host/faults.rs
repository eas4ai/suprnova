//! Closed, server-owned fault schedules for reference-host scenarios.

/// One compiled deterministic fault schedule.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReferenceFaultSchedule {
    /// No controlled failure is applied.
    #[default]
    None,
    /// The first asynchronous delivery creates one deterministic sequence gap.
    SequenceGapOnce,
    /// The first upload body is interrupted after its first bounded chunk.
    UploadBodyInterruptedOnce,
}
