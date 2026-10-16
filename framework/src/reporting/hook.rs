//! Panic hooks only capture and enqueue. Application callbacks run on a worker,
//! where a panicking callback cannot abort a process inside its panic hook.

use super::event::CapturedEvent;
use super::pending::Pending;
use super::{Reporter, ReportingConfig};
use crate::{Exceptions, FrameworkError};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Duration;

struct State {
    reporter: Arc<dyn Reporter>,
    config: ReportingConfig,
}
struct Message {
    reporter: Arc<dyn Reporter>,
    event: CapturedEvent,
    message: String,
}
struct Queue {
    sender: SyncSender<Message>,
    pending: Arc<Pending>,
}

static STATE: RwLock<Option<Arc<State>>> = RwLock::new(None);
static QUEUE: OnceLock<Queue> = OnceLock::new();
static INSTALL: Mutex<()> = Mutex::new(());
static DROPPED: AtomicU64 = AtomicU64::new(0);

thread_local! { static HANDLING: Cell<bool> = const { Cell::new(false) }; }

pub(crate) fn install(
    reporter: Arc<dyn Reporter>,
    config: ReportingConfig,
) -> Result<(), FrameworkError> {
    let _install = INSTALL.lock().unwrap_or_else(PoisonError::into_inner);
    if QUEUE.get().is_none() {
        let (sender, receiver) = sync_channel::<Message>(128);
        let pending = Arc::new(Pending::default());
        let worker_pending = pending.clone();
        std::thread::Builder::new()
            .name("suprnova-panic-reports".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    HANDLING.set(true);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let error = FrameworkError::internal(message.message);
                        Exceptions::report_captured(
                            &error,
                            message.event.resolve(),
                            &*message.reporter,
                        )
                    }));
                    HANDLING.set(false);
                    if !matches!(result, Ok(Ok(()))) {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                    worker_pending.finish();
                }
            })
            .map_err(|_| FrameworkError::internal("could not start the panic reporting worker"))?;
        QUEUE.set(Queue { sender, pending }).map_err(|_| {
            FrameworkError::internal("panic reporting worker was already installed")
        })?;
    }
    *STATE.write().unwrap_or_else(PoisonError::into_inner) =
        Some(Arc::new(State { reporter, config }));
    crate::error::install_reporting_hook();
    Ok(())
}

pub(crate) fn capture(info: &std::panic::PanicHookInfo<'_>) {
    if HANDLING.try_with(Cell::get).unwrap_or(true) {
        return;
    }
    let Some(queue) = QUEUE.get() else {
        return;
    };
    let state = STATE.try_read().ok().and_then(|state| state.clone());
    let Some(state) = state else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let message = info
        .payload_as_str()
        .unwrap_or("panic with non-string payload")
        .to_owned();
    let location = info.location().map(ToString::to_string);
    let Some(event) = CapturedEvent::from_hook(&message, location.as_deref(), &state.config) else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    queue.pending.begin();
    if queue
        .sender
        .try_send(Message {
            reporter: state.reporter.clone(),
            event,
            message,
        })
        .is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        queue.pending.finish();
    }
}

pub(crate) fn flush(timeout: Duration) -> bool {
    QUEUE.get().is_none_or(|queue| queue.pending.wait(timeout))
}
pub(crate) fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}
