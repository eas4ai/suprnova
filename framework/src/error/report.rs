//! The error report a framework response carries in process.
//!
//! When the framework turns a failure into a response, the body says only
//! what a client may see: a 5xx body is a generic message. The report
//! keeps what a developer needs - the error and its source chain, or the
//! panic message and where the panic was raised. It rides in the
//! response's in-process extensions and never in a header or the body, so
//! it reaches `TestResponse` and anything else that calls `handle_request`
//! in process, and never the wire.
//!
//! Laravel keeps a test request's exceptions in a process-wide
//! `LoggedExceptionCollection`. A report on the response itself belongs to
//! exactly one request, so two requests in flight in one process never
//! mix their reports.

use std::any::{Any, TypeId};
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Once, PoisonError, RwLock};

use futures::FutureExt;

use super::frames::{self, RecordedFrames};

/// What went wrong in one request: kept in process for the developer, never
/// sent to the client.
///
/// The framework attaches one to the response it builds from a failure -
/// a handler or middleware error converted through
/// `From<FrameworkError> for HttpResponse`, a panic caught by the panic
/// boundary, or a failure one of the framework's own middleware answers
/// with a 5xx it builds itself, such as a session store that cannot write.
/// Read it with
/// [`TestResponse::error_report`](crate::testing::TestResponse::error_report)
/// in a test, or [`HttpResponse::error_report`](crate::HttpResponse::error_report)
/// in middleware. A failing `TestResponse` assertion prints it.
///
/// `Display` renders an error's chain one link per line, outermost first,
/// each source on a `caused by:` line. A panic renders as
/// `panicked at <file:line:column>: <message>`.
///
/// With debug mode on, a report built while the server serves a request
/// also holds the stack frames recorded where the error was created or
/// the panic was raised. The development error page lists them. Two
/// reports are equal when they report the same failure; the frames and
/// the type name are not compared.
#[derive(Debug, Clone)]
pub struct ErrorReport {
    kind: Kind,
    /// See [`Self::type_name`].
    type_name: Option<&'static str>,
    /// Where the error was created or the panic raised. `None` with debug
    /// off, and for an error created outside the request's own task.
    frames: Option<RecordedFrames>,
    source_frames: Vec<(String, RecordedFrames)>,
}

impl PartialEq for ErrorReport {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for ErrorReport {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Error {
        chain: Vec<String>,
    },
    Panic {
        message: String,
        location: Option<String>,
    },
}

impl ErrorReport {
    /// Report `error` and its `source()` chain.
    ///
    /// A link that only repeats the end of the link before it is skipped,
    /// the rule [`render_error_chain`](crate::render_error_chain) follows:
    /// [`FrameworkError::from_external`](crate::FrameworkError::from_external)
    /// copies its source's message, so a plain walk would report that text
    /// twice.
    ///
    /// The frames are the ones recorded in this request under the error's
    /// message, when there are any. Repeated messages take successive
    /// records, newest first, before the display text is deduplicated.
    ///
    /// `error` is `'static` so the report can tell a `FrameworkError` from
    /// another error and name its type; see [`Self::type_name`].
    pub(crate) fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        let mut chain = vec![error.to_string()];
        let frames = frames::recorded_for(&chain[0], 0);
        // Errors are recorded inner first, outer last. Count every source,
        // even when its message is omitted from the displayed chain.
        let mut occurrences = HashMap::from([(chain[0].clone(), 1usize)]);
        let mut source_frames = Vec::new();
        let mut current = error.source();
        while let Some(source) = current {
            let link = source.to_string();
            let rank = occurrences.entry(link.clone()).or_default();
            if let Some(recorded) = frames::recorded_for(&link, *rank) {
                source_frames.push((link.clone(), recorded));
            }
            *rank += 1;
            let repeats_previous = chain
                .last()
                .is_some_and(|previous| previous.ends_with(&link));
            if !link.is_empty() && !repeats_previous {
                chain.push(link);
            }
            current = source.source();
        }
        let type_name = match error.downcast_ref::<super::FrameworkError>() {
            Some(framework) => Some(framework.type_name()),
            None => wrapped_type_name(error),
        };
        Self {
            kind: Kind::Error { chain },
            type_name,
            frames,
            source_frames,
        }
    }

    /// Report an error response that carries no report of its own: the
    /// router's `404` for a path no route matches, or a middleware's own
    /// `{"message": ...}` answer. `message` is the response's message, or
    /// its status's reason phrase.
    ///
    /// Built only for the Inertia error callback to read; it is never
    /// attached to the response.
    pub(crate) fn from_message(message: String) -> Self {
        Self {
            kind: Kind::Error {
                chain: vec![message],
            },
            type_name: None,
            frames: None,
            source_frames: Vec::new(),
        }
    }

    /// Report a panic the panic boundary caught. `location` and `frames`
    /// are `None` when they were not captured; see [`catch_panic`].
    pub(crate) fn from_panic(
        message: String,
        location: Option<String>,
        frames: Option<RecordedFrames>,
    ) -> Self {
        Self {
            kind: Kind::Panic { message, location },
            type_name: Some("panic"),
            frames,
            source_frames: Vec::new(),
        }
    }

    /// The stack frames recorded where the error was created or the panic
    /// raised, for the development error page.
    pub(crate) fn frames(&self) -> Option<&RecordedFrames> {
        self.frames.as_ref()
    }

    /// Each source error's own trace, independent of the outer conversion's trace.
    pub(crate) fn source_frames(&self) -> &[(String, RecordedFrames)] {
        &self.source_frames
    }

    /// The error's own message, then the message of each `source()` in
    /// order. For a panic, the panic message alone.
    pub fn chain(&self) -> &[String] {
        match &self.kind {
            Kind::Error { chain } => chain,
            Kind::Panic { message, .. } => std::slice::from_ref(message),
        }
    }

    /// The type of what failed, as the development error page names it
    /// beside the headline, the way Laravel's page heads with the
    /// exception class:
    ///
    /// - for an error built by
    ///   [`FrameworkError::from_external`](crate::FrameworkError::from_external)
    ///   or
    ///   [`from_external_with`](crate::FrameworkError::from_external_with),
    ///   the path `std::any::type_name` gives for the error it wraps, such
    ///   as `std::io::error::Error` for an `std::io::Error`;
    /// - for any other `FrameworkError`, `FrameworkError::<Variant>`, such
    ///   as `FrameworkError::ModelNotFound`;
    /// - for a panic, `panic`.
    ///
    /// `None` when the type is not known: a report the framework built
    /// from a response's message alone, or from an error of another type
    /// that no `from_external` call has wrapped.
    pub fn type_name(&self) -> Option<&str> {
        self.type_name
    }

    /// Whether the panic boundary caught a panic, as opposed to an error
    /// becoming a response.
    pub fn is_panic(&self) -> bool {
        matches!(self.kind, Kind::Panic { .. })
    }

    /// Where the panic was raised, as `file:line:column`. `None` for an
    /// error, and for a panic whose location was not captured: one raised
    /// again with `std::panic::resume_unwind`, which skips the panic hook,
    /// or one raised after the application replaced the panic hook.
    pub fn panic_location(&self) -> Option<&str> {
        match &self.kind {
            Kind::Panic { location, .. } => location.as_deref(),
            Kind::Error { .. } => None,
        }
    }
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            Kind::Error { chain } => {
                let mut links = chain.iter();
                if let Some(first) = links.next() {
                    f.write_str(first)?;
                }
                for link in links {
                    write!(f, "\ncaused by: {link}")?;
                }
                Ok(())
            }
            Kind::Panic {
                message,
                location: Some(location),
            } => write!(f, "panicked at {location}: {message}"),
            Kind::Panic {
                message,
                location: None,
            } => write!(f, "panicked: {message}"),
        }
    }
}

/// One error type [`FrameworkError::from_external`] or `from_external_with`
/// has wrapped: its name, and a check that a type-erased error is of it.
///
/// [`FrameworkError::from_external`]: crate::FrameworkError::from_external
struct WrappedType {
    id: TypeId,
    name: &'static str,
    is: fn(&(dyn std::error::Error + 'static)) -> bool,
}

/// Every error type the process has wrapped in `FrameworkError::External`.
///
/// The variant keeps its source as `Arc<dyn Error>`, which forgets the
/// concrete type's name, and it cannot gain a field for the name without
/// breaking every `match` an application wrote against it. So each
/// wrapping constructor adds its type here, once, and a report finds the
/// name by asking each entry whether the source is of its type. The list
/// grows with the number of distinct types the program wraps, which is
/// fixed when it compiles, and never with the number of errors.
///
/// A push-only list whose critical sections are all code of this module,
/// so a poisoned lock is read as it is.
static WRAPPED_TYPES: RwLock<Vec<WrappedType>> = RwLock::new(Vec::new());

/// Add `E` to [`WRAPPED_TYPES`] unless it is there already.
pub(crate) fn remember_wrapped_type<E>()
where
    E: std::error::Error + 'static,
{
    let id = TypeId::of::<E>();
    let known = |types: &[WrappedType]| types.iter().any(|wrapped| wrapped.id == id);
    if known(&WRAPPED_TYPES.read().unwrap_or_else(PoisonError::into_inner)) {
        return;
    }
    let mut types = WRAPPED_TYPES
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if !known(&types) {
        types.push(WrappedType {
            id,
            name: std::any::type_name::<E>(),
            is: is_of::<E>,
        });
    }
}

/// Whether `error` is an `E`.
fn is_of<E>(error: &(dyn std::error::Error + 'static)) -> bool
where
    E: std::error::Error + 'static,
{
    error.is::<E>()
}

/// The type path of `error`, when its type is in [`WRAPPED_TYPES`].
pub(crate) fn wrapped_type_name(error: &(dyn std::error::Error + 'static)) -> Option<&'static str> {
    WRAPPED_TYPES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|wrapped| (wrapped.is)(error))
        .map(|wrapped| wrapped.name)
}

/// A panic [`catch_panic`] caught, with where it was raised when known.
pub(crate) struct CaughtPanic {
    /// The panic payload, as `catch_unwind` returned it.
    pub(crate) payload: Box<dyn Any + Send>,
    /// `file:line:column` of the panic, when the hook recorded it.
    pub(crate) location: Option<String>,
    /// The stack of the panic, when the hook recorded it: only while the
    /// request records frames, which it does with debug mode on.
    pub(crate) frames: Option<RecordedFrames>,
}

/// What the panic hook saw of the last panic.
struct HookedPanic {
    /// The message, when the payload was a string.
    message: Option<String>,
    location: String,
    frames: Option<RecordedFrames>,
}

tokio::task_local! {
    /// The last panic raised while the current request's chain was being
    /// polled: its message as the hook saw it, its location, and its
    /// stack when the request records frames.
    ///
    /// Task-local and not thread-local: the slot exists only while this
    /// request's own future is polled, so a panic on another task that
    /// shares the worker thread can never write into it.
    static LAST_PANIC: Cell<Option<HookedPanic>>;
}

static INSTALL_LOCATION_HOOK: Once = Once::new();

/// Wrap the process panic hook, once, so a panic raised while a request's
/// chain is polled records its location, and with debug on its stack, for
/// that request.
///
/// `catch_unwind` hands back the payload but not the location; only the
/// panic hook sees that. The wrapper calls the hook it replaced on every
/// panic, so the default report on stderr, or an application's own hook
/// installed before this, keeps running unchanged.
fn install_location_hook() {
    INSTALL_LOCATION_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(location) = info.location() {
                // Off a request's task - another task, a blocking pool
                // thread, a plain thread - there is no slot, and the panic
                // is no request's to report.
                let _ = LAST_PANIC.try_with(|slot| {
                    slot.set(Some(HookedPanic {
                        message: info.payload_as_str().map(str::to_owned),
                        location: location.to_string(),
                        frames: frames::capture_for_panic(location),
                    }));
                });
            }
            previous(info);
        }));
    });
}

/// Poll `future` to completion, catching a panic and the location it was
/// raised at, and its stack when the request records frames.
///
/// The location and stack are kept only when the message the hook
/// recorded matches the payload that unwound here. A panic the request caught itself and a
/// later `resume_unwind` of a different payload, which does not run the
/// hook, would otherwise pair the new message with the old location.
pub(crate) async fn catch_panic<F: Future>(future: F) -> Result<F::Output, CaughtPanic> {
    install_location_hook();
    LAST_PANIC
        .scope(Cell::new(None), async {
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(output) => Ok(output),
                Err(payload) => {
                    let hooked = LAST_PANIC
                        .with(Cell::take)
                        .filter(|hooked| hooked.message.as_deref() == payload_text(&*payload));
                    let (location, frames) = match hooked {
                        Some(hooked) => (Some(hooked.location), hooked.frames),
                        None => (None, None),
                    };
                    Err(CaughtPanic {
                        payload,
                        location,
                        frames,
                    })
                }
            }
        })
        .await
}

/// The panic message, when the payload is the `&str` or `String` that
/// `panic!` produces.
fn payload_text(payload: &(dyn Any + Send)) -> Option<&str> {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}

#[cfg(test)]
mod tests {
    //! The rules of the report itself: how a chain is walked, how each
    //! kind renders, and when a panic's location is kept.

    use super::*;

    /// One link of an error chain, with the rest of the chain as its
    /// source.
    #[derive(Debug)]
    struct Link {
        message: &'static str,
        source: Option<Box<Link>>,
    }

    impl fmt::Display for Link {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for Link {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_deref()
                .map(|source| source as &(dyn std::error::Error + 'static))
        }
    }

    /// The chain `messages`, outermost first.
    fn chain(messages: &[&'static str]) -> Link {
        let (first, rest) = messages.split_first().expect("a chain has a first link");
        Link {
            message: first,
            source: (!rest.is_empty()).then(|| Box::new(chain(rest))),
        }
    }

    #[test]
    fn an_error_reports_each_source_outermost_first_one_per_line() {
        let report = ErrorReport::from_error(&chain(&[
            "posting the invoice failed",
            "writing ledger entry 42 failed",
            "disk /var/ledger is full",
        ]));

        assert_eq!(
            report.chain(),
            [
                "posting the invoice failed",
                "writing ledger entry 42 failed",
                "disk /var/ledger is full",
            ]
        );
        assert_eq!(
            report.to_string(),
            "posting the invoice failed\n\
             caused by: writing ledger entry 42 failed\n\
             caused by: disk /var/ledger is full"
        );
        assert!(!report.is_panic());
        assert_eq!(report.panic_location(), None);
    }

    #[test]
    fn a_link_that_only_repeats_the_end_of_the_one_before_it_is_skipped() {
        // The wrapper's message ends with its source's, the way
        // `from_external` and a `format!("...: {e}")` wrapper build one.
        // The link after the repeat is still compared, and kept.
        let report = ErrorReport::from_error(&chain(&[
            "fetching rates: the feed answered 503",
            "the feed answered 503",
            "connection reset by peer",
        ]));

        assert_eq!(
            report.chain(),
            [
                "fetching rates: the feed answered 503",
                "connection reset by peer"
            ]
        );
    }

    #[test]
    fn an_empty_link_is_skipped() {
        let report =
            ErrorReport::from_error(&chain(&["sending the payout failed", "", "bank offline"]));

        assert_eq!(
            report.chain(),
            ["sending the payout failed", "bank offline"]
        );
    }

    #[test]
    fn a_panic_renders_its_location_when_known_and_its_message_alone_when_not() {
        let located = ErrorReport::from_panic(
            "ledger index page 7 is unreadable".to_string(),
            Some("src/ledger.rs:31:9".to_string()),
            None,
        );
        assert!(located.is_panic());
        assert_eq!(located.chain(), ["ledger index page 7 is unreadable"]);
        assert_eq!(located.panic_location(), Some("src/ledger.rs:31:9"));
        assert_eq!(
            located.to_string(),
            "panicked at src/ledger.rs:31:9: ledger index page 7 is unreadable"
        );

        let unlocated =
            ErrorReport::from_panic("ledger index page 7 is unreadable".to_string(), None, None);
        assert_eq!(unlocated.panic_location(), None);
        assert_eq!(
            unlocated.to_string(),
            "panicked: ledger index page 7 is unreadable"
        );
    }

    #[tokio::test]
    async fn catch_panic_keeps_the_location_of_the_panic_that_unwound() {
        let line = line!() + 2;
        let caught = catch_panic(async {
            panic!("ledger index page 7 is unreadable");
        })
        .await;

        let panic = caught.expect_err("the panic must be caught");
        assert_eq!(
            payload_text(&*panic.payload),
            Some("ledger index page 7 is unreadable")
        );
        let location = panic
            .location
            .expect("the hook must have recorded the location");
        assert!(
            location.starts_with(&format!("{}:{line}:", file!())),
            "the location must name {}:{line}; got {location}",
            file!()
        );
    }

    #[tokio::test]
    async fn catch_panic_drops_a_location_recorded_for_a_different_panic() {
        // The future catches its own panic, which records a location,
        // then unwinds with another payload through `resume_unwind`,
        // which runs no hook. The recorded location belongs to the first
        // panic, so the report must not pair it with the second.
        let caught = catch_panic(async {
            let _ = std::panic::catch_unwind(|| panic!("the cursor was stale"));
            std::panic::resume_unwind(Box::new("the ledger feed closed".to_string()));
        })
        .await;

        let panic = caught.expect_err("the resumed unwind must be caught");
        assert_eq!(
            payload_text(&*panic.payload),
            Some("the ledger feed closed")
        );
        assert_eq!(panic.location, None);
    }
}
