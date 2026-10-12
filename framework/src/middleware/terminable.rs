//! Terminable middleware - Laravel-style post-response hooks.
//!
//! In Laravel, a middleware class with a `terminate($request, $response)`
//! method is invoked after the response has been sent to the client. The
//! canonical use case is `StartSession::terminate` which writes the
//! mutated session back to disk once the response is in flight, keeping
//! the slow IO off the response path.
//!
//! Suprnova ships the equivalent as a dedicated [`Terminable`] trait. A
//! middleware that wants post-response work implements `Terminable` and
//! gets registered separately from its `handle` registration -
//! [`Middleware`] and [`Terminable`] are orthogonal so the request-path
//! and the termination-path stay clearly typed.
//!
//! Termination runs after [`crate::server::Server`] hands the response
//! to hyper. The server iterates the registered [`Terminable`]
//! implementations in registration order and awaits each one. A hook
//! that panics is logged via `tracing::error!` and the hooks after it
//! still run - the response has already left the building, so there's
//! nobody left to surface the failure to. A graceful shutdown waits a
//! bounded time for the hooks still running.
//!
//! [`Middleware`]: crate::middleware::Middleware

use crate::http::HttpResponse;
use async_trait::async_trait;
use std::any::TypeId;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tokio::task::JoinSet;

/// A middleware-shaped post-response hook.
///
/// Implementers don't have to also implement [`crate::middleware::Middleware`] -
/// the two surfaces are independent. A type that wants both must
/// implement both traits and register itself in both places.
///
/// The `terminate` receiver borrows `&self` so the hook can be
/// registered as `Arc<dyn Terminable>` and reused across requests
/// without per-request allocation. `request_method`, `request_path`,
/// and the response status / headers / body are the post-response
/// snapshot - there is no live `Request` body here because hyper has
/// already streamed it to the client.
#[async_trait]
pub trait Terminable: Send + Sync {
    /// Run post-response work. The response has already been sent, so
    /// there is no error to return; a panic is logged and the hooks after
    /// this one still run.
    async fn terminate(&self, snapshot: &TerminationSnapshot);
}

/// Immutable snapshot of the request+response handed to every
/// [`Terminable::terminate`] call.
///
/// Carries the wire-level method and path plus the final
/// [`HttpResponse`] status code so most hooks (session persisters,
/// audit loggers, metrics emitters) can act without reaching into
/// hyper internals. Hooks that need more should snapshot the data they
/// need inside their `Middleware::handle` and stash it in their own
/// state instead of widening this struct.
#[derive(Debug, Clone)]
pub struct TerminationSnapshot {
    /// The HTTP method of the request being terminated.
    pub method: hyper::Method,
    /// The path of the request being terminated.
    pub path: String,
    /// Final response status code seen by the client.
    pub status: u16,
}

impl TerminationSnapshot {
    /// Build a snapshot from a borrowed method, path, and final
    /// [`HttpResponse`].
    pub fn from_response(method: hyper::Method, path: &str, response: &HttpResponse) -> Self {
        Self {
            method,
            path: path.to_string(),
            status: response.status_code(),
        }
    }
}

/// Stored shape of the terminable registry - extracted to a `type`
/// alias so the static declaration below doesn't trip
/// `clippy::type_complexity`.
type TerminableMap = Vec<(TypeId, Arc<dyn Terminable>)>;

/// Process-global terminable registry. Keyed by `TypeId` so
/// registration is idempotent per concrete type, matching the
/// idempotency contract of [`crate::middleware::register_global_middleware`].
static TERMINABLE_REGISTRY: OnceLock<RwLock<TerminableMap>> = OnceLock::new();

fn registry_lock() -> &'static RwLock<TerminableMap> {
    TERMINABLE_REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Register a terminable hook. Idempotent per concrete type - the
/// second registration of a given `T: Terminable` is dropped with a
/// debug log, the same way [`crate::middleware::register_global_middleware`]
/// handles its double-registration case.
pub fn register_terminable<T: Terminable + 'static>(terminable: T) {
    let tid = TypeId::of::<T>();
    let lock = registry_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guard.iter().any(|(existing, _)| *existing == tid) {
        tracing::debug!(
            "terminable of this type is already registered; skipping the duplicate. \
             Wrap it in a distinct newtype to register multiple instances."
        );
        return;
    }
    guard.push((tid, Arc::new(terminable)));
}

/// All registered terminable hooks (snapshot of the `Arc`s). The
/// server iterates this list after every response, in registration
/// order, awaiting each `terminate` call.
pub fn registered_terminables() -> Vec<Arc<dyn Terminable>> {
    let lock = registry_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().map(|(_, t)| t.clone()).collect()
}

/// Number of terminables currently registered.
pub fn terminable_count() -> usize {
    let lock = registry_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.len()
}

/// Whether a terminable of this concrete type has been registered.
pub fn has_terminable<T: Terminable + 'static>() -> bool {
    let tid = TypeId::of::<T>();
    let lock = registry_lock();
    let guard = match lock.read() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.iter().any(|(existing, _)| *existing == tid)
}

/// Run every registered terminable's `terminate` against the given
/// snapshot, in registration order. Used by the server after a response
/// is sent.
///
/// Each hook runs inside its own panic boundary. A hook that panics is
/// logged and the hooks after it still run: they were all awaited in one
/// task with no boundary, so one panic unwound the batch and every later
/// hook - an audit record, a metric - was skipped without a trace.
pub async fn dispatch_termination(snapshot: TerminationSnapshot) {
    let hooks = registered_terminables();
    for hook in hooks {
        let run = std::panic::AssertUnwindSafe(hook.terminate(&snapshot));
        if let Err(payload) = futures::FutureExt::catch_unwind(run).await {
            let message = payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a non-string panic payload".to_string());
            tracing::error!(
                method = %snapshot.method,
                path = %snapshot.path,
                panic = %message,
                "a terminable hook panicked; the hooks after it still run"
            );
        }
    }
}

/// The termination batches still running, one task per response.
///
/// The server spawns each batch so the client gets its response at once,
/// and it used to forget the task: a graceful shutdown drained the
/// connections, the WebSocket handlers, the supervisors and the queued
/// listeners, and then let the runtime's teardown cut off a hook that was
/// still writing its audit record. Holding the tasks here lets
/// [`drain_terminations`] wait for them.
static TERMINATIONS: OnceLock<Mutex<JoinSet<()>>> = OnceLock::new();

fn terminations() -> std::sync::MutexGuard<'static, JoinSet<()>> {
    // A poisoned set is still a set of tasks: recover it rather than
    // lose the hooks it holds (a hot-path registry, per the lock policy).
    TERMINATIONS
        .get_or_init(|| Mutex::new(JoinSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Spawn the termination batch for one response, in the request's
/// container scope, and keep its task for the shutdown drain.
pub(crate) fn spawn_termination(snapshot: TerminationSnapshot) {
    let mut tasks = terminations();
    // Reap the batches that already finished, so the set holds only
    // running ones however many responses the server sends.
    while tasks.try_join_next().is_some() {}
    tasks.spawn(crate::container::App::in_current_scope(async move {
        dispatch_termination(snapshot).await;
    }));
}

/// Wait up to `grace` for the termination batches still running, then
/// abort the rest. Returns how many were aborted.
///
/// Called by the server's graceful shutdown once the connections are
/// drained, so no response can start another batch behind it. Bounded
/// like every other drain there: a hook that never returns must not keep
/// the process from exiting.
pub(crate) async fn drain_terminations(grace: std::time::Duration) -> usize {
    let mut tasks = std::mem::take(&mut *terminations());
    if tasks.is_empty() {
        return 0;
    }
    let deadline = tokio::time::sleep(grace);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            joined = tasks.join_next() => {
                if joined.is_none() {
                    return 0;
                }
            }
            _ = &mut deadline => {
                let abandoned = tasks.len();
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return abandoned;
            }
        }
    }
}

/// Wipe every registered terminable. Test-only convenience.
#[doc(hidden)]
pub fn clear_terminables_for_test() {
    let lock = registry_lock();
    let mut guard = match lock.write() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as TokioMutex;

    /// Tests touch the process-global registry, so they share an async
    /// mutex. A `std::sync::Mutex` would trigger `clippy::await_holding_lock`
    /// because the guard lives across the `dispatch_termination` await.
    static SERIAL: TokioMutex<()> = TokioMutex::const_new(());

    struct Counter {
        hits: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Terminable for Counter {
        async fn terminate(&self, _snapshot: &TerminationSnapshot) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Other {
        hits: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Terminable for Other {
        async fn terminate(&self, _snapshot: &TerminationSnapshot) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn snapshot() -> TerminationSnapshot {
        TerminationSnapshot {
            method: hyper::Method::GET,
            path: "/foo".to_string(),
            status: 200,
        }
    }

    #[tokio::test]
    async fn registers_and_dispatches_in_order() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();

        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        register_terminable(Counter { hits: c1.clone() });
        register_terminable(Other { hits: c2.clone() });
        assert_eq!(terminable_count(), 2);

        dispatch_termination(snapshot()).await;
        assert_eq!(c1.load(Ordering::SeqCst), 1);
        assert_eq!(c2.load(Ordering::SeqCst), 1);

        clear_terminables_for_test();
    }

    #[tokio::test]
    async fn duplicate_registration_keyed_by_type_is_idempotent() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();

        let c1 = Arc::new(AtomicUsize::new(0));
        register_terminable(Counter { hits: c1.clone() });
        register_terminable(Counter { hits: c1.clone() });
        assert_eq!(terminable_count(), 1);

        dispatch_termination(snapshot()).await;
        assert_eq!(c1.load(Ordering::SeqCst), 1);

        clear_terminables_for_test();
    }

    #[tokio::test]
    async fn has_terminable_reflects_registration_state() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();
        assert!(!has_terminable::<Counter>());
        register_terminable(Counter {
            hits: Arc::new(AtomicUsize::new(0)),
        });
        assert!(has_terminable::<Counter>());
        assert!(!has_terminable::<Other>());
        clear_terminables_for_test();
    }

    struct Panics;
    #[async_trait]
    impl Terminable for Panics {
        async fn terminate(&self, _snapshot: &TerminationSnapshot) {
            panic!("a terminable that fails on purpose");
        }
    }

    /// A hook that panics is reported, and the hooks registered after it
    /// still run. The hooks used to be awaited one after the other with no
    /// boundary, so the panic unwound the whole batch and skipped every
    /// later hook, such as an audit record or a metric.
    #[tokio::test]
    async fn a_panicking_hook_does_not_skip_the_hooks_after_it() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();
        let after = Arc::new(AtomicUsize::new(0));
        register_terminable(Panics);
        register_terminable(Counter {
            hits: after.clone(),
        });

        let outcome = tokio::spawn(dispatch_termination(snapshot())).await;
        clear_terminables_for_test();

        assert!(outcome.is_ok(), "the panic stays inside the dispatch");
        assert_eq!(after.load(Ordering::SeqCst), 1, "the later hook ran");
    }

    struct Slow {
        done: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Terminable for Slow {
        async fn terminate(&self, _snapshot: &TerminationSnapshot) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            self.done.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct NeverEnds;
    #[async_trait]
    impl Terminable for NeverEnds {
        async fn terminate(&self, _snapshot: &TerminationSnapshot) {
            std::future::pending::<()>().await;
        }
    }

    /// The shutdown drain waits for the hooks of responses already sent.
    /// The batch used to be spawned and forgotten, so the runtime's
    /// teardown cut off a hook that was still writing its record.
    #[tokio::test]
    async fn the_shutdown_drain_waits_for_a_running_hook() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();
        let done = Arc::new(AtomicUsize::new(0));
        register_terminable(Slow { done: done.clone() });

        spawn_termination(snapshot());
        let abandoned = drain_terminations(std::time::Duration::from_secs(5)).await;
        clear_terminables_for_test();

        assert_eq!(abandoned, 0, "the hook finished inside the grace");
        assert_eq!(done.load(Ordering::SeqCst), 1, "the drain waited for it");
    }

    /// A hook that outlives the grace is aborted and counted, so the drain
    /// is bounded and the operator learns what was cut off.
    #[tokio::test]
    async fn the_shutdown_drain_aborts_a_hook_past_its_grace() {
        let _g = SERIAL.lock().await;
        clear_terminables_for_test();
        register_terminable(NeverEnds);

        spawn_termination(snapshot());
        let abandoned = drain_terminations(std::time::Duration::from_millis(50)).await;
        clear_terminables_for_test();

        assert_eq!(abandoned, 1, "the hook past its grace was aborted");
    }

    #[test]
    fn termination_snapshot_from_response() {
        let resp = HttpResponse::text("ok").status(201);
        let snap = TerminationSnapshot::from_response(hyper::Method::POST, "/users", &resp);
        assert_eq!(snap.method, hyper::Method::POST);
        assert_eq!(snap.path, "/users");
        assert_eq!(snap.status, 201);
    }
}
