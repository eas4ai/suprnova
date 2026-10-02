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

use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Once;

use futures::FutureExt;

/// What went wrong in one request: kept in process for the developer, never
/// sent to the client.
///
/// The framework attaches one to every response it builds from an error -
/// a handler or middleware error converted through
/// `From<FrameworkError> for HttpResponse`, or a panic caught by the panic
/// boundary. Read it with
/// [`TestResponse::error_report`](crate::testing::TestResponse::error_report)
/// in a test, or [`HttpResponse::error_report`](crate::HttpResponse::error_report)
/// in middleware. A failing `TestResponse` assertion prints it.
///
/// `Display` renders an error's chain one link per line, outermost first,
/// each source on a `caused by:` line. A panic renders as
/// `panicked at <file:line:column>: <message>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    kind: Kind,
}

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
    pub(crate) fn from_error(error: &dyn std::error::Error) -> Self {
        let mut chain = vec![error.to_string()];
        let mut current = error.source();
        while let Some(source) = current {
            let link = source.to_string();
            let repeats_previous = chain
                .last()
                .is_some_and(|previous| previous.ends_with(&link));
            if !link.is_empty() && !repeats_previous {
                chain.push(link);
            }
            current = source.source();
        }
        Self {
            kind: Kind::Error { chain },
        }
    }

    /// Report a panic the panic boundary caught. `location` is `None` when
    /// it was not captured; see [`catch_panic`].
    pub(crate) fn from_panic(message: String, location: Option<String>) -> Self {
        Self {
            kind: Kind::Panic { message, location },
        }
    }

    /// The error's own message, then the message of each `source()` in
    /// order. For a panic, the panic message alone.
    pub fn chain(&self) -> &[String] {
        match &self.kind {
            Kind::Error { chain } => chain,
            Kind::Panic { message, .. } => std::slice::from_ref(message),
        }
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

/// A panic [`catch_panic`] caught, with where it was raised when known.
pub(crate) struct CaughtPanic {
    /// The panic payload, as `catch_unwind` returned it.
    pub(crate) payload: Box<dyn Any + Send>,
    /// `file:line:column` of the panic, when the hook recorded it.
    pub(crate) location: Option<String>,
}

tokio::task_local! {
    /// The last panic raised while the current request's chain was being
    /// polled: its message as the hook saw it, and its location.
    ///
    /// Task-local and not thread-local: the slot exists only while this
    /// request's own future is polled, so a panic on another task that
    /// shares the worker thread can never write into it.
    static LAST_PANIC: Cell<Option<(Option<String>, String)>>;
}

static INSTALL_LOCATION_HOOK: Once = Once::new();

/// Wrap the process panic hook, once, so a panic raised while a request's
/// chain is polled records its location for that request.
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
                    slot.set(Some((
                        info.payload_as_str().map(str::to_owned),
                        location.to_string(),
                    )));
                });
            }
            previous(info);
        }));
    });
}

/// Poll `future` to completion, catching a panic and the location it was
/// raised at.
///
/// The location is kept only when the message the hook recorded matches
/// the payload that unwound here. A panic the request caught itself and a
/// later `resume_unwind` of a different payload, which does not run the
/// hook, would otherwise pair the new message with the old location.
pub(crate) async fn catch_panic<F: Future>(future: F) -> Result<F::Output, CaughtPanic> {
    install_location_hook();
    LAST_PANIC
        .scope(Cell::new(None), async {
            match AssertUnwindSafe(future).catch_unwind().await {
                Ok(output) => Ok(output),
                Err(payload) => {
                    let location = LAST_PANIC.with(Cell::take).and_then(|(message, location)| {
                        (message.as_deref() == payload_text(&*payload)).then_some(location)
                    });
                    Err(CaughtPanic { payload, location })
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
