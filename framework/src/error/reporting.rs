//! Reporting errors, and the errors that end a queued job's retries
//! (PAR-111).
//!
//! Laravel sends every error it does not answer itself through one place,
//! `Handler::report`: the exceptions of HTTP requests, of failed queue
//! jobs and of console commands. An application hooks that place with
//! `reportable` callbacks, and names with `dontRetry` the errors that end
//! a job's retries. [`Exceptions`] is that place here.
//!
//! # What is reported
//!
//! - Every `FrameworkError` that `From<FrameworkError> for HttpResponse`
//!   turns into a `5xx` response. The `ErrorOccurred` event still follows.
//! - Every error the framework answers with a `5xx` response it builds
//!   itself, such as the `500` of a changed session the store could not
//!   write.
//! - Every failed attempt of a queued job: a handler that returns `Err`
//!   or panics, an attempt that runs past its timeout, and an error
//!   `FailOnException` fails the job for.
//! - Every error a console command returns, except
//!   [`FrameworkError::AlreadyReported`], which says the user has seen it,
//!   and the exit code of [`FrameworkError::exit`], which the command
//!   chose, as a Laravel command returns an integer that never reaches the
//!   handler.
//! - Every error the application hands to [`Exceptions::report`].
//!
//! A report writes the `framework error` log line after the callbacks. A
//! `5xx` site that has already logged its failure, with fields that line
//! does not carry, reports through `Exceptions::report_logged` instead,
//! which skips it, so one failure is logged once.
//!
//! # The registry is process-wide
//!
//! An application registers its callbacks once, in `bootstrap`, and every
//! request, worker and command of the process reports through them. A
//! test that registers a callback shares the process with the other tests
//! of its binary under plain `cargo test`. The hidden [`Exceptions::reset`]
//! empties the registry; a test calls it at its start and end while it
//! holds its suite's environment lock and runs `#[serial]`, and gives its
//! callbacks error types or messages no other test uses. Under nextest
//! every test is a process of its own.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use super::FrameworkError;

tokio::task_local! { static SUPPRESSED: bool; }

/// A callback that runs the application's closure when an error is of the
/// closure's type, and says whether it did.
type Run = dyn Fn(&FrameworkError) -> bool + Send + Sync;

/// A predicate that says whether an error ends a job's retries.
type DontRetry = dyn Fn(&FrameworkError) -> bool + Send + Sync;

/// One `reportable` registration.
struct Reporter {
    run: Box<Run>,
    /// Set by [`ReportableHandler::stop`] after registration, so it is
    /// shared with the handle the application holds.
    stop: AtomicBool,
}

/// The process's reporters and its don't-retry list.
#[derive(Default)]
struct Registry {
    reporters: Vec<Arc<Reporter>>,
    dont_retry: Vec<Arc<DontRetry>>,
    dont_report: Vec<Arc<DontRetry>>,
    deduplicate: bool,
    reported: Vec<(usize, String, std::time::Instant)>,
}

/// Every critical section only pushes, clones or clears a list, all code
/// of this module: the lists are whole after any panic, so a poisoned lock
/// is read as it is (the rule `crate::lock` states for such values). The
/// application's closures run after the guard is dropped.
static REGISTRY: RwLock<Registry> = RwLock::new(Registry {
    reporters: Vec::new(),
    dont_retry: Vec::new(),
    dont_report: Vec::new(),
    deduplicate: false,
    reported: Vec::new(),
});

/// The application's error reporting: Laravel's `Exceptions` facade over
/// the exception handler.
///
/// ```rust,no_run
/// use suprnova::{Exceptions, FrameworkError};
///
/// // In bootstrap.rs: send every reported I/O error to an alerting hook,
/// // and keep it out of the default log.
/// Exceptions::reportable(|error: &std::io::Error| {
///     eprintln!("disk trouble: {error}");
/// })
/// .stop();
///
/// // A declined card will be declined again: fail the job at once.
/// Exceptions::dont_retry_when(|error: &FrameworkError| error.status_code() == 402);
/// ```
pub struct Exceptions;

impl Exceptions {
    /// Preserve a caught panic's payload and location through normal callbacks.
    pub fn report_panic(message: &str, location: Option<&str>) {
        crate::reporting::event::with_panic(message, location, || {
            Self::report(&FrameworkError::internal(message));
        });
    }

    /// Register `callback` to run for every reported error of type `E`,
    /// before the default log line.
    ///
    /// The closure's parameter type decides which errors it receives, as
    /// a Laravel closure's type hint does:
    ///
    /// - `|error: &FrameworkError|` receives every reported error.
    /// - `|error: &E|` for any other error type receives an error whose
    ///   wrapped source, the error handed to
    ///   [`FrameworkError::from_external`] or
    ///   [`FrameworkError::from_external_with`], is an `E`.
    ///
    /// Only the wrapped source itself is compared, not the errors in its
    /// own `source()` chain: Laravel compares the thrown exception's class
    /// and not its previous exceptions. To act on a cause deeper in the
    /// chain, take `&FrameworkError` and walk `std::error::Error::source`.
    ///
    /// Callbacks run in the order they were registered. A callback that
    /// panics is logged and reporting goes on, so a broken reporter cannot
    /// take a queue worker down. Call [`ReportableHandler::stop`] on the
    /// returned handle to end reporting after this callback for an error
    /// it receives. Dropping the handle keeps the callback registered.
    pub fn reportable<E, F>(callback: F) -> ReportableHandler
    where
        E: std::error::Error + 'static,
        F: Fn(&E) + Send + Sync + 'static,
    {
        let reporter = Arc::new(Reporter {
            run: Box::new(move |error: &FrameworkError| match matching::<E>(error) {
                Some(matched) => {
                    callback(matched);
                    true
                }
                None => false,
            }),
            stop: AtomicBool::new(false),
        });
        write_registry(|registry| registry.reporters.push(Arc::clone(&reporter)));
        ReportableHandler { reporter }
    }

    /// Report `error`: run the callbacks that receive it, in registration
    /// order, then write the `framework error` log line at `error` level,
    /// unless a callback marked with [`ReportableHandler::stop`] received
    /// it.
    ///
    /// Use it for an error your code handles itself but still wants
    /// recorded, Laravel's `report($e)`. The log line carries the status
    /// the error maps to, its full source chain and the request id when
    /// there is one, as the line a `5xx` response writes.
    ///
    /// [`FrameworkError::AlreadyReported`] is not reported: it says the
    /// user has seen the failure already.
    pub fn report(error: &FrameworkError) {
        if let Err(failure) = Self::try_report(error) {
            tracing::warn!(error = %failure, "error reporter rejected an event");
        }
    }

    pub(crate) fn without_reporting<T>(f: impl FnOnce() -> T) -> T {
        SUPPRESSED.sync_scope(true, f)
    }

    /// Surface a custom driver's failure to explicit callers without failing HTTP paths.
    pub fn try_report(error: &FrameworkError) -> Result<(), FrameworkError> {
        if !Self::run_callbacks(error) {
            return Ok(());
        }
        crate::Reporting::report(error)?;
        let request_id = crate::logging::current_request_id().map(|id| id.as_str().to_string());
        tracing::error!(
            status = error.status_code(),
            error = %super::render_error_chain(error),
            request_id = ?request_id,
            "framework error"
        );
        Ok(())
    }

    /// Report `error`, a failure its call site has already written its own
    /// log line for: the callbacks run as [`Self::report`] runs them, and
    /// the `framework error` line is not written (PAR-111).
    ///
    /// For the `5xx` responses the framework builds itself whose site logs
    /// the failure with fields the default line does not carry, such as the
    /// key of a rate limiter that fails closed or the route of a Live
    /// request that could not be prepared. Writing both lines logged one
    /// failure twice, on every request of an outage. The site's line is
    /// written before the report, whatever a callback does, as it was before
    /// these sites reported, so a callback marked with
    /// [`ReportableHandler::stop`] stops the later callbacks and not that
    /// line.
    pub(crate) fn report_logged(error: &FrameworkError) {
        if Self::run_callbacks(error) {
            let _ = crate::Reporting::report(error);
        }
    }

    pub(crate) fn report_captured(
        error: &FrameworkError,
        event: crate::ReportingEvent,
        reporter: &dyn crate::Reporter,
    ) -> Result<(), FrameworkError> {
        if Self::run_callbacks(error) {
            reporter.report(event)?;
        }
        Ok(())
    }

    /// Run the callbacks that receive `error`, in registration order, and
    /// say whether the default log line is still due: not for a silent
    /// error, and not for one a stopping callback received.
    fn run_callbacks(error: &FrameworkError) -> bool {
        if SUPPRESSED.try_with(|value| *value).unwrap_or(false)
            || error.is_silent()
            || crate::reporting::already_reported_engine_error(error)
            || error.status_code() < 500
        {
            return false;
        }
        let predicates = read_registry(|registry| registry.dont_report.clone());
        if predicates
            .iter()
            .any(|predicate| catch_unwind(AssertUnwindSafe(|| predicate(error))).unwrap_or(false))
        {
            return false;
        }
        let key = (std::ptr::from_ref(error).addr(), error.to_string());
        let now = std::time::Instant::now();
        let window = crate::App::get::<crate::ReportingConfig>()
            .unwrap_or_default()
            .duplicate_window;
        let duplicate = {
            let mut registry = REGISTRY.write().unwrap_or_else(PoisonError::into_inner);
            if registry.deduplicate {
                registry
                    .reported
                    .retain(|(_, _, reported)| now.duration_since(*reported) < window);
                if registry
                    .reported
                    .iter()
                    .any(|(address, message, _)| (*address, message) == (key.0, &key.1))
                {
                    true
                } else {
                    if registry.reported.len() >= 4096 {
                        registry.reported.remove(0);
                    }
                    registry.reported.push((key.0, key.1, now));
                    false
                }
            } else {
                false
            }
        };
        if duplicate {
            return false;
        }
        let reporters = read_registry(|registry| registry.reporters.clone());
        for reporter in &reporters {
            match catch_unwind(AssertUnwindSafe(|| (reporter.run)(error))) {
                Ok(true) if reporter.stop.load(Ordering::Acquire) => return false,
                Ok(_) => {}
                Err(payload) => tracing::error!(
                    panic = %panic_text(&*payload),
                    "a reportable callback panicked; the error is reported on"
                ),
            }
        }
        true
    }

    /// Suppress an expected error type before callbacks and tracker delivery.
    pub fn dont_report<E: std::error::Error + 'static>() {
        Self::dont_report_when(|error| matching::<E>(error).is_some());
    }

    /// Suppress expected failures without changing how they are answered.
    pub fn dont_report_when<F: Fn(&FrameworkError) -> bool + Send + Sync + 'static>(predicate: F) {
        write_registry(|registry| registry.dont_report.push(Arc::new(predicate)));
    }

    /// Suppress repeated reports within the configured duplicate window, including callbacks.
    pub fn dont_report_duplicates() {
        write_registry(|registry| registry.deduplicate = true);
    }

    /// End a queued job's retries when it fails with an error of type
    /// `E`: the job fails at once, whatever attempts it has left.
    ///
    /// `E` matches as it does for [`Self::reportable`]: `FrameworkError`
    /// matches every error, and any other type matches an error whose
    /// wrapped source is an `E`. Laravel's `dontRetry`.
    pub fn dont_retry<E>()
    where
        E: std::error::Error + 'static,
    {
        let predicate: Arc<DontRetry> =
            Arc::new(|error: &FrameworkError| matching::<E>(error).is_some());
        write_registry(|registry| registry.dont_retry.push(predicate));
    }

    /// End a queued job's retries when `predicate` returns `true` for the
    /// error it failed with. Laravel's `dontRetryWhen`.
    ///
    /// A predicate that panics counts as `false`, and the panic is logged.
    pub fn dont_retry_when<F>(predicate: F)
    where
        F: Fn(&FrameworkError) -> bool + Send + Sync + 'static,
    {
        let predicate: Arc<DontRetry> = Arc::new(predicate);
        write_registry(|registry| registry.dont_retry.push(predicate));
    }

    /// Whether `error` ends a queued job's retries: a type registered with
    /// [`Self::dont_retry`] matches it, or a predicate registered with
    /// [`Self::dont_retry_when`] returns `true`. The worker asks this
    /// before it retries a failed attempt, as Laravel's worker asks
    /// `shouldStopRetries`.
    pub fn should_stop_retries(error: &FrameworkError) -> bool {
        let predicates = read_registry(|registry| registry.dont_retry.clone());
        predicates.iter().any(|predicate| {
            match catch_unwind(AssertUnwindSafe(|| predicate(error))) {
                Ok(stop) => stop,
                Err(payload) => {
                    tracing::error!(
                        panic = %panic_text(&*payload),
                        "a dont_retry_when predicate panicked; it counts as false"
                    );
                    false
                }
            }
        })
    }

    /// Forget every callback and don't-retry entry. For tests only: see
    /// the module docs for how a test isolates the registry.
    #[doc(hidden)]
    pub fn reset() {
        write_registry(|registry| *registry = Registry::default());
    }
}

/// The registration [`Exceptions::reportable`] returns: Laravel's
/// `ReportableHandler`.
pub struct ReportableHandler {
    reporter: Arc<Reporter>,
}

impl ReportableHandler {
    /// Stop reporting after this callback for an error it receives: the
    /// callbacks registered after it and the default log line do not run
    /// for that error. An error this callback does not receive is
    /// reported as before.
    pub fn stop(self) -> Self {
        self.reporter.stop.store(true, Ordering::Release);
        self
    }
}

/// `error` as an `E`: the error itself when `E` is `FrameworkError`, else
/// its wrapped source when that is an `E`.
fn matching<E>(error: &FrameworkError) -> Option<&E>
where
    E: std::error::Error + 'static,
{
    (error as &dyn Any)
        .downcast_ref::<E>()
        .or_else(|| error.external_source()?.downcast_ref::<E>())
}

fn read_registry<T>(read: impl FnOnce(&Registry) -> T) -> T {
    read(&REGISTRY.read().unwrap_or_else(PoisonError::into_inner))
}

fn write_registry(write: impl FnOnce(&mut Registry)) {
    write(&mut REGISTRY.write().unwrap_or_else(PoisonError::into_inner));
}

/// The text of a panic payload, when it is the `&str` or `String` that
/// `panic!` produces.
fn panic_text(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic with a payload that is not text")
}
