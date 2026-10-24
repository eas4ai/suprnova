//! Error tracker drivers share one application binding, so switching trackers
//! does not change exception handlers or application listeners.

mod config;
mod drivers;
mod engine;
mod helpers;
pub(crate) mod hook;
mod logs;
mod pending;
pub(crate) mod scope;
#[cfg(feature = "reporting-sentry")]
mod transport;
pub(crate) use engine::{
    already_reported_engine_error, capture_engine_reports, report_engine_panic,
    reported_engine_error,
};
pub(crate) mod event;

pub use config::{ReportingConfig, ReportingDriver};
#[cfg(feature = "reporting-sentry")]
pub use drivers::SentryReporter;
pub use drivers::{LogReporter, NullReporter};
pub use event::{ReportingEvent, ReportingKind};
pub use helpers::{report, report_if, report_unless, rescue};

use crate::{App, FrameworkError};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static SEND_FAILURES: AtomicU64 = AtomicU64::new(0);
use std::time::Duration;

/// Delivery counters make dropped reports visible without logging secrets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReportingMetrics {
    /// Envelopes accepted by the bounded sender.
    pub queued: u64,
    /// Envelopes acknowledged by the tracker.
    pub delivered: u64,
    /// Envelopes rejected by capacity, timeout or network failure.
    pub dropped: u64,
    /// Repeated incidents suppressed inside the duplicate window.
    pub duplicates: u64,
}

/// A driver receives owned, scrubbed events without borrowing a request.
pub trait Reporter: Send + Sync + 'static {
    /// Queue an event without waiting for the tracker, keeping failures local.
    fn report(&self, event: ReportingEvent) -> Result<(), FrameworkError>;

    /// Custom drivers receive complete events while queued drivers can defer symbol resolution.
    fn report_error(&self, error: &FrameworkError) -> Result<(), FrameworkError> {
        self.report(ReportingEvent::from_error(error))
    }

    /// Drain accepted events within a deadline before a process exits.
    fn flush(&self, timeout: Duration) -> Result<bool, FrameworkError>;

    /// Expose delivery loss without requiring a recursive error log.
    fn metrics(&self) -> ReportingMetrics {
        ReportingMetrics::default()
    }
}

/// The container binding used by HTTP requests, commands and background work.
pub struct Reporting;

impl Reporting {
    /// Bind a configured driver once the application's configuration is ready.
    pub fn configure(config: ReportingConfig) -> Result<(), FrameworkError> {
        let reporter = config.build()?;
        App::instance(config.clone());
        App::bind::<dyn Reporter>(reporter.clone());
        hook::install(reporter, config)?;
        suprnova_live::set_panic_observer(Arc::new(report_engine_panic))
            .map_err(|_| FrameworkError::internal("could not register the Live panic reporter"))?;
        logs::install();
        Ok(())
    }

    /// Honor an application binding before choosing the environment's driver.
    pub fn bootstrap() -> Result<(), FrameworkError> {
        if !App::has_binding::<dyn Reporter>() {
            Self::configure(ReportingConfig::from_env()?)?;
        }
        let config = App::get::<ReportingConfig>().unwrap_or_default();
        hook::install(Self::reporter()?, config)?;
        suprnova_live::set_panic_observer(Arc::new(report_engine_panic))
            .map_err(|_| FrameworkError::internal("could not register the Live panic reporter"))?;
        logs::install();
        Ok(())
    }

    /// Deliver through the application binding without requiring a listener.
    pub fn report(error: &FrameworkError) -> Result<(), FrameworkError> {
        if !App::has_binding::<dyn Reporter>() {
            Self::bootstrap()?;
        }
        let reporter = Self::reporter()?;
        Self::call_driver(|| reporter.report_error(error))
    }

    /// Give a daemon's pending reports a bounded opportunity to finish.
    pub fn flush(timeout: Duration) -> Result<bool, FrameworkError> {
        if !App::has_binding::<dyn Reporter>() {
            return Ok(true);
        }
        let started = std::time::Instant::now();
        if !hook::flush(timeout) {
            return Ok(false);
        }
        Self::reporter()?.flush(timeout.saturating_sub(started.elapsed()))
    }

    /// Inspect delivery loss without exposing an event or tracker credentials.
    pub fn metrics() -> Result<ReportingMetrics, FrameworkError> {
        let mut metrics = Self::reporter()?.metrics();
        metrics.dropped = metrics
            .dropped
            .saturating_add(hook::dropped())
            .saturating_add(SEND_FAILURES.load(Ordering::Relaxed));
        Ok(metrics)
    }

    pub(crate) fn send(event: ReportingEvent) -> Result<(), FrameworkError> {
        let reporter = Self::reporter()?;
        Self::send_using(&reporter, event)
    }

    pub(crate) fn send_using(
        reporter: &Arc<dyn Reporter>,
        event: ReportingEvent,
    ) -> Result<(), FrameworkError> {
        Self::call_driver(|| reporter.report(event))
    }

    fn call_driver(
        call: impl FnOnce() -> Result<(), FrameworkError>,
    ) -> Result<(), FrameworkError> {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).unwrap_or_else(|_| {
                Err(FrameworkError::internal(
                    "the error reporting driver panicked",
                ))
            });
        if result.is_err() {
            SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn reporter() -> Result<Arc<dyn Reporter>, FrameworkError> {
        App::resolve_make::<dyn Reporter>()
    }
}
