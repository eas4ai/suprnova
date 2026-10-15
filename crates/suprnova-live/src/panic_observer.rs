//! Host reporting for panics behind the engine's redacted error boundary.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::panic::{AssertUnwindSafe, UnwindSafe, catch_unwind};
use std::sync::{Arc, PoisonError, RwLock};

/// Whether the panic ended engine work or only its diagnostic callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanicKind {
    /// An engine operation failed because application or host code panicked.
    Operation,
    /// A non-authoritative tracing or metrics callback panicked; work continues.
    Observer,
}

type Observer = dyn Fn(&str, PanicKind) + Send + Sync;
static OBSERVER: RwLock<Option<Arc<Observer>>> = RwLock::new(None);

/// The process-wide panic observer could not be installed.
#[derive(Debug)]
pub struct PanicObserverError;

impl fmt::Display for PanicObserverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Live panic observer registry is poisoned")
    }
}

impl Error for PanicObserverError {}

/// Replace the process's observer for panics caught by the Live engine.
///
/// The engine keeps its errors safe for browser responses. A host installs
/// this observer to report the original panic message on the server without
/// adding it to those errors. The callback runs after the unwind is caught;
/// it must queue its work promptly. The kind distinguishes a failed operation
/// from a diagnostic observer that panicked while work continued. A callback
/// panic is contained too.
///
/// Returns an error if the observer registry is poisoned.
pub fn set_panic_observer(
    observer: Arc<dyn Fn(&str, PanicKind) + Send + Sync>,
) -> Result<(), PanicObserverError> {
    let previous = {
        let mut slot = OBSERVER.write().map_err(|_| PanicObserverError)?;
        slot.replace(observer)
    };
    drop(previous);
    Ok(())
}

/// Every engine boundary reports the payload before mapping it to a closed
/// failure kind. The reporting callback itself uses the standard catcher,
/// so a broken observer cannot recursively report its own panic.
pub(crate) fn catch_unwind_reported<F, T>(operation: F) -> std::thread::Result<T>
where
    F: FnOnce() -> T + UnwindSafe,
{
    catch_unwind(operation).inspect_err(|payload| report(payload.as_ref(), PanicKind::Operation))
}

/// Diagnostic observers do not determine whether engine work succeeded.
pub(crate) fn catch_unwind_observed<F, T>(operation: F) -> std::thread::Result<T>
where
    F: FnOnce() -> T + UnwindSafe,
{
    catch_unwind(operation).inspect_err(|payload| report(payload.as_ref(), PanicKind::Observer))
}

fn report(payload: &(dyn Any + Send), kind: PanicKind) {
    // Only whole Arc values are installed under the lock; callbacks and
    // replaced observers are dropped outside it, so recovery keeps it whole.
    let observer = OBSERVER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(observer) = observer {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let _ = catch_unwind(AssertUnwindSafe(|| observer(message, kind)));
    }
}
