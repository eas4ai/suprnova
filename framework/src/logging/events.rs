//! `MessageLogged`: what every write that reaches a channel reports, to the
//! callbacks [`Log::listen`](super::Log::listen) adds and through the
//! application's event dispatcher, as Laravel's `Logger` dispatches it.

use super::channel::LogLevel;
use crate::events::EventFacade;
use serde_json::{Map, Value};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

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

/// Report `event`: the callbacks first, on this thread, then the event
/// dispatcher.
///
/// Logging is synchronous and the dispatcher is async, so the event is
/// dispatched on a task spawned on the current Tokio runtime, and the write
/// never waits for a listener. Outside a runtime there is nothing to spawn
/// on: the callbacks still run, and the dispatcher does not hear the write.
/// When the events fake is active on this thread the event is recorded
/// before this returns, so a test reads it without racing a task. A
/// callback that panics, and a listener that fails, are logged and never
/// fail the write.
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
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(DISPATCHING.scope((), async move {
        if let Err(error) = EventFacade::dispatch_best_effort(event).await {
            tracing::warn!(error = %error, "a MessageLogged listener failed");
        }
    }));
}
