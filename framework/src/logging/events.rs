//! `MessageLogged`: what every write that reaches a channel reports, to the
//! callbacks [`Log::listen`](super::Log::listen) adds and through the
//! application's event dispatcher, as Laravel's `Logger` dispatches it.

use super::channel::LogLevel;
use crate::events::EventFacade;
use serde_json::{Map, Value};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

/// A write that reached a channel, as Laravel's `MessageLogged` reports it:
/// a profiler or a test collects a request's lines with it.
///
/// Each write reports it once, however many channels its logger writes to,
/// with the message before its `{key}` placeholders are replaced.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MessageLogged {
    /// The write's level.
    pub level: LogLevel,
    /// The message, before its `{key}` placeholders are replaced.
    pub message: String,
    /// The context: what the scope shares, the logger's, and the call's, or
    /// a `tracing` event's fields and those of the spans it is in.
    pub context: Map<String, Value>,
}

impl MessageLogged {
    pub(crate) fn new(level: LogLevel, message: String, context: Map<String, Value>) -> Self {
        Self {
            level,
            message,
            context,
        }
    }
}

impl crate::events::Event for MessageLogged {
    fn event_name() -> &'static str {
        "MessageLogged"
    }
}

type Callback = Arc<dyn Fn(&MessageLogged) + Send + Sync>;

static CALLBACKS: RwLock<Vec<Callback>> = RwLock::new(Vec::new());

/// Set by the first [`listen`], so a write with no callback reads one flag
/// instead of a lock.
static HAS_CALLBACKS: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Set while this thread runs the callbacks or records into the events
    /// fake, so a write they make is not reported again.
    static REPORTING: Cell<bool> = const { Cell::new(false) };
}

tokio::task_local! {
    /// Set on the task that dispatches a `MessageLogged`: the dispatcher
    /// logs each dispatch, and a listener may log, and neither line may
    /// dispatch another `MessageLogged`, which would never end.
    static DISPATCHING: ();
}

pub(crate) fn listen(callback: Callback) {
    CALLBACKS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .push(callback);
    HAS_CALLBACKS.store(true, Ordering::Release);
}

/// Whether a write now would be reported to anything: a callback, a
/// listener, the events fake. A write that nothing observes builds no event,
/// so ordinary `tracing` traffic pays for two checks. A write made while a
/// `MessageLogged` is being reported is not reported again.
pub(crate) fn observed() -> bool {
    if REPORTING.get() || DISPATCHING.try_with(|_| ()).is_ok() {
        return false;
    }
    HAS_CALLBACKS.load(Ordering::Acquire) || EventFacade::is_observed::<MessageLogged>()
}

/// Puts back the thread's reporting flag, however the callbacks end.
struct Reporting(bool);

impl Reporting {
    fn enter() -> Self {
        Self(REPORTING.replace(true))
    }
}

impl Drop for Reporting {
    fn drop(&mut self) {
        REPORTING.set(self.0);
    }
}

/// The runtime that dispatches the writes made outside any Tokio runtime: a
/// current-thread runtime that a thread of its own drives for the life of
/// the process. The first such write starts it. `None` when the runtime or
/// its thread could not be made.
static DETACHED: OnceLock<Option<tokio::runtime::Handle>> = OnceLock::new();

/// The detached runtime, started on first use.
///
/// A failure to start it is logged once, after the lock is released, and
/// every later write outside a runtime reaches only the callbacks.
fn detached_runtime() -> Option<&'static tokio::runtime::Handle> {
    let mut failure = None;
    let runtime = DETACHED
        .get_or_init(|| match start_detached_runtime() {
            Ok(handle) => Some(handle),
            Err(error) => {
                failure = Some(error);
                None
            }
        })
        .as_ref();
    if let Some(error) = failure {
        tracing::warn!(
            error = %error,
            "the MessageLogged dispatch thread did not start; a write outside a runtime reaches only the Log::listen callbacks"
        );
    }
    runtime
}

/// Build the detached runtime here and move it to a thread that drives it
/// forever, so the caller gets its handle without waiting for the thread.
fn start_detached_runtime() -> std::io::Result<tokio::runtime::Handle> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    std::thread::Builder::new()
        .name("suprnova-message-logged".to_owned())
        .spawn(move || runtime.block_on(std::future::pending::<()>()))?;
    Ok(handle)
}

/// Report `event`: the callbacks first, on this thread, then the event
/// dispatcher.
///
/// Logging is synchronous and the dispatcher is async, so the event is
/// dispatched on a spawned task, and the write never waits for a listener.
/// The task runs on the current Tokio runtime. A write made outside any
/// runtime, on a plain thread for example, hands its task to the detached
/// runtime instead, so the dispatcher hears it too. The task runs under
/// `DISPATCHING`, so a line the dispatcher or a listener logs is not
/// reported again. When the events fake is active on this thread the event
/// is recorded before this returns, so a test reads it without racing a
/// task. A callback that panics, and a listener that fails, are logged and
/// never fail the write.
pub(crate) fn report(event: MessageLogged) {
    let _reporting = Reporting::enter();
    if HAS_CALLBACKS.load(Ordering::Acquire) {
        let callbacks = CALLBACKS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for callback in callbacks {
            // The panic hook has already printed the panic; the write goes on.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(&event)));
        }
    }
    if !EventFacade::is_observed::<MessageLogged>() {
        return;
    }
    if crate::events::testing::is_active::<MessageLogged>() {
        crate::events::testing::record(event);
        return;
    }
    let dispatch = DISPATCHING.scope((), async move {
        if let Err(error) = EventFacade::dispatch_best_effort(event).await {
            tracing::warn!(error = %error, "a MessageLogged listener failed");
        }
    });
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(dispatch);
        }
        Err(_) => {
            if let Some(runtime) = detached_runtime() {
                runtime.spawn(dispatch);
            }
        }
    }
}
