//! Container scopes: the unit of work a scoped binding lives for.
//!
//! A scoped binding (`App::scoped`, `App::bind_scoped`) is registered once,
//! like any other binding, but its value belongs to one unit of work. The
//! framework opens a scope for each HTTP request, WebSocket session, queue
//! job attempt, queued event listener attempt, workflow run, supervisor
//! run, scheduled task and console command. The first resolution inside a
//! scope runs the factory, later resolutions in the same scope return that
//! value, and the value is dropped with the scope.
//!
//! The scope is a task-local, so it follows its future across worker
//! threads and does not reach a bare `tokio::spawn`. `App::in_current_scope`
//! and `App::spawn_scoped` carry it into another task, which then shares
//! the scope's values; the scope ends when the last future that holds it
//! ends. `App::run_scoped` opens a scope for work the framework does not
//! run itself.
//!
//! Test overrides live in their own task-local and thread-local (see
//! [`super::testing`]), so a scope never hides a test fake and never drops
//! one when it ends.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::thread::{self, ThreadId};

use hyper::body::{Body, Frame, SizeHint};
use tokio::task::futures::TaskLocalFuture;

/// A value a scoped factory built, type-erased the way the container stores
/// every value.
type ScopedValue = Arc<dyn Any + Send + Sync>;

/// One binding's value in one scope. A binding with no slot has not been
/// resolved in the scope yet, or the last build of it panicked.
enum Slot {
    /// The factory runs on the thread it holds.
    Building(ThreadId),
    /// The value, built once for the scope.
    Ready(ScopedValue),
}

/// What a resolver finds for its binding.
enum Found {
    /// No slot: the resolver builds the value.
    Empty,
    /// Another resolver builds the value.
    Building,
    /// The value.
    Ready(ScopedValue),
}

/// What [`ScopeState::claim`] hands a resolver.
enum Claim {
    /// The value, built earlier in the scope or by the resolver waited for.
    Ready(ScopedValue),
    /// The slot is this resolver's to build.
    Build,
    /// Waiting for the slot would close a cycle of scoped factories.
    Cycle,
}

/// The slots of one scope and the resolvers waiting on them, guarded by the
/// scope's mutex.
#[derive(Default)]
struct ScopeInner {
    /// The slot of every binding resolved in the scope, keyed like the
    /// container's bindings.
    values: HashMap<TypeId, Slot>,
    /// For each thread that waits in this scope, the binding it waits for.
    waiting: HashMap<ThreadId, TypeId>,
}

impl ScopeInner {
    /// What a resolver of `key` finds.
    fn found(&self, key: TypeId) -> Found {
        match self.values.get(&key) {
            None => Found::Empty,
            Some(Slot::Building(_)) => Found::Building,
            Some(Slot::Ready(value)) => Found::Ready(Arc::clone(value)),
        }
    }

    /// Whether `me` waiting for `key` would close a cycle.
    ///
    /// The chain is: the thread that builds `key`, the binding that thread
    /// waits for, the thread that builds that one, and so on. When it
    /// reaches a binding that `me` builds, the threads on it wait for each
    /// other and none would ever finish. A binding `me` builds itself is
    /// the chain of length one: a factory that resolves its own binding.
    fn closes_a_cycle(&self, key: TypeId, me: ThreadId) -> bool {
        let mut wanted = key;
        // Each step moves to a thread that waits, and no waiting thread
        // closed a cycle when it began to wait, so a chain that does not
        // reach `me` ends within `waiting.len() + 1` steps.
        for _ in 0..=self.waiting.len() {
            let Some(Slot::Building(builder)) = self.values.get(&wanted) else {
                return false;
            };
            if *builder == me {
                return true;
            }
            match self.waiting.get(builder) {
                Some(next) => wanted = *next,
                None => return false,
            }
        }
        false
    }
}

/// The identity the next scope takes. Ids only tell scopes apart, so the
/// counter needs no ordering with anything else.
static NEXT_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

/// The values of one scope.
///
/// The mutex is held to read or change the slots and never while a
/// factory runs. A resolver that finds its binding being built by another
/// thread waits on `changed`, which blocks its thread until the value is
/// ready. The check that the wait closes no cycle and the record of the
/// wait happen under the one mutex, so two threads can never both decide
/// to wait on each other.
pub(crate) struct ScopeState {
    /// Tells this scope from every other scope of the process. It is taken
    /// from a counter, so a scope that ended never shares its id with a
    /// later one.
    id: u64,
    inner: Mutex<ScopeInner>,
    changed: Condvar,
}

impl Default for ScopeState {
    fn default() -> Self {
        Self {
            id: NEXT_SCOPE_ID.fetch_add(1, Ordering::Relaxed),
            inner: Mutex::default(),
            changed: Condvar::default(),
        }
    }
}

impl ScopeState {
    /// Lock the slots.
    fn lock(&self) -> MutexGuard<'_, ScopeInner> {
        // Recover in place from a poisoned lock, as the container does: no
        // user code runs while it is held, so the maps are intact.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Find the value of `key` for the thread `me`: take a ready value,
    /// take the slot to build it, wait while another thread builds it, or
    /// refuse a wait that would close a cycle.
    fn claim(&self, key: TypeId, me: ThreadId) -> Claim {
        let mut inner = self.lock();
        loop {
            match inner.found(key) {
                Found::Ready(value) => {
                    inner.waiting.remove(&me);
                    return Claim::Ready(value);
                }
                Found::Empty => {
                    inner.waiting.remove(&me);
                    inner.values.insert(key, Slot::Building(me));
                    return Claim::Build;
                }
                Found::Building => {
                    if inner.closes_a_cycle(key, me) {
                        inner.waiting.remove(&me);
                        return Claim::Cycle;
                    }
                    inner.waiting.insert(me, key);
                    inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
    }

    /// Store the value `key`'s builder produced and wake the resolvers
    /// waiting for it.
    fn finish(&self, key: TypeId, value: ScopedValue) {
        self.lock().values.insert(key, Slot::Ready(value));
        self.changed.notify_all();
    }

    /// Empty the slot of a build `builder` did not finish, and wake the
    /// resolvers waiting for it: one of them builds the value instead.
    fn abandon(&self, key: TypeId, builder: ThreadId) {
        let mut inner = self.lock();
        if matches!(inner.values.get(&key), Some(Slot::Building(owner)) if *owner == builder) {
            inner.values.remove(&key);
            drop(inner);
            self.changed.notify_all();
        }
    }
}

/// Abandons the build of one slot when dropped, unless the value was
/// stored: a factory that panics leaves the slot empty and wakes the
/// resolvers waiting for it.
struct BuildGuard<'a> {
    state: &'a ScopeState,
    key: TypeId,
    builder: ThreadId,
}

impl Drop for BuildGuard<'_> {
    fn drop(&mut self) {
        self.state.abandon(self.key, self.builder);
    }
}

tokio::task_local! {
    /// The container scope of the unit of work the current task runs.
    static CURRENT_SCOPE: Arc<ScopeState>;
}

/// A handle on one container scope.
///
/// Cloning the handle shares the scope. Its values live until the last
/// handle, and the last future running in the scope, is gone.
#[derive(Clone, Default)]
pub(crate) struct ContainerScope(Arc<ScopeState>);

impl ContainerScope {
    /// A new, empty scope. It allocates the scope and nothing per binding:
    /// a slot is created only when a scoped binding is resolved in it.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The scope the current task runs in, or `None` outside any scope.
    pub(crate) fn current() -> Option<Self> {
        CURRENT_SCOPE.try_with(Arc::clone).ok().map(Self)
    }

    /// The identity of the scope the current task runs in, or `None` outside
    /// any scope. Two units of work that each opened a scope have different
    /// ids, so code that keeps state for one unit of work can tell whether
    /// the caller is that unit of work.
    pub(crate) fn current_id() -> Option<u64> {
        Self::current().map(|scope| scope.id())
    }

    /// The identity of this scope, unique in the process and stable for the
    /// scope's life. Clones of a handle share it.
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }

    /// Run `fut` inside this scope.
    pub(crate) fn run<F>(self, fut: F) -> TaskLocalFuture<Arc<ScopeState>, F>
    where
        F: std::future::Future,
    {
        CURRENT_SCOPE.scope(self.0, fut)
    }

    /// Wrap `body` so that each poll of it runs inside this scope, which
    /// then lives until the body ends or is dropped.
    pub(crate) fn body<B>(self, body: B) -> ScopedBody<B> {
        ScopedBody {
            scope: self.0,
            inner: body,
        }
    }
}

/// Run `fut` inside a new scope of its own: the shape of every unit of work
/// the framework runs.
pub(crate) fn run_in_new_scope<F>(fut: F) -> TaskLocalFuture<Arc<ScopeState>, F>
where
    F: std::future::Future,
{
    ContainerScope::new().run(fut)
}

/// Why a scoped binding could not be resolved.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ScopedError {
    /// No container scope is active on the current task.
    NoScope {
        /// The requested type, as `std::any::type_name` spells it.
        type_name: &'static str,
    },
    /// The resolution closes a cycle of scoped factories: the value is
    /// being built by a factory that, directly or through others, waits
    /// for this resolution.
    Cycle {
        /// The requested type, as `std::any::type_name` spells it.
        type_name: &'static str,
    },
}

impl fmt::Display for ScopedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoScope { type_name } => write!(
                f,
                "`{type_name}` is a scoped binding and no container scope is active: it \
                 resolves only inside a unit of work the framework runs (a request, a \
                 WebSocket session, a job, a scheduled task, a command), inside \
                 App::run_scoped, or in a task that carries a scope (App::spawn_scoped, \
                 App::in_current_scope)"
            ),
            Self::Cycle { type_name } => write!(
                f,
                "`{type_name}` is a scoped binding in a dependency cycle: it was resolved \
                 while being built, by its own factory or through the factories of other \
                 scoped bindings, in this task or in another task that shares the scope"
            ),
        }
    }
}

impl From<ScopedError> for crate::error::FrameworkError {
    #[track_caller]
    fn from(error: ScopedError) -> Self {
        Self::internal(error.to_string())
    }
}

/// Collapse a scoped-resolution error to `None` for the resolvers that
/// return an `Option`, and log it: the `Option` cannot carry the reason,
/// and a silent `None` would hide a caller's mistake.
pub(crate) fn or_log<T>(result: Result<Option<T>, ScopedError>) -> Option<T> {
    result.unwrap_or_else(|error| {
        tracing::warn!(error = %error, "scoped binding could not be resolved");
        None
    })
}

/// Resolve the scoped binding `key` in the current scope: the value built
/// earlier in this scope, or a new one from `factory`.
///
/// No lock is held while `factory` runs, so it may resolve other bindings,
/// scoped ones included. A resolver that finds the binding being built by
/// another thread blocks its thread until the value is ready, so the value
/// is built at most once. A resolution that would close a cycle of scoped
/// factories, in one task or across tasks that share the scope, gets
/// [`ScopedError::Cycle`] instead of waiting or recursing without end.
pub(crate) fn resolve(
    key: TypeId,
    type_name: &'static str,
    factory: &(dyn Fn() -> ScopedValue + Send + Sync),
) -> Result<ScopedValue, ScopedError> {
    let state = CURRENT_SCOPE
        .try_with(Arc::clone)
        .map_err(|_| ScopedError::NoScope { type_name })?;
    let me = thread::current().id();
    match state.claim(key, me) {
        Claim::Ready(value) => Ok(value),
        Claim::Cycle => Err(ScopedError::Cycle { type_name }),
        Claim::Build => {
            let _guard = BuildGuard {
                state: &state,
                key,
                builder: me,
            };
            let value = factory();
            state.finish(key, Arc::clone(&value));
            Ok(value)
        }
    }
}

/// A response body produced in the container scope of its request.
///
/// The server polls a streamed body after the request's future has
/// returned. Each poll enters the scope, so the stream resolves the
/// request's scoped values, and the body keeps the scope until it ends or
/// is dropped.
pub(crate) struct ScopedBody<B> {
    scope: Arc<ScopeState>,
    inner: B,
}

impl<B> Body for ScopedBody<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let scope = Arc::clone(&this.scope);
        CURRENT_SCOPE.sync_scope(scope, || Pin::new(&mut this.inner).poll_frame(cx))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    //! The scope primitives on their own, without the `App` facade.
    //!
    //! The tests with several threads enter one scope from each thread, as
    //! tasks that share a scope do on a multi-thread runtime. Each waits for
    //! the threads' results with a bound, so a deadlock fails the test
    //! instead of hanging it.
    use super::*;
    use std::panic::AssertUnwindSafe;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, mpsc};
    use std::time::Duration;

    /// Run `f` on this thread inside `state`, as a task of the scope would.
    fn in_scope<R>(state: &Arc<ScopeState>, f: impl FnOnce() -> R) -> R {
        CURRENT_SCOPE.sync_scope(Arc::clone(state), f)
    }

    /// The next message of `receiver`. A cap, not a measurement: a thread
    /// that never sends fails the test instead of hanging it.
    fn bounded<T>(receiver: &mpsc::Receiver<T>) -> T {
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the thread finished instead of hanging")
    }

    /// Return once a resolver waits in `state`. A factory calls this to hold
    /// its build open until another thread waits for it.
    fn until_a_resolver_waits(state: &ScopeState) {
        while state.lock().waiting.is_empty() {
            thread::yield_now();
        }
    }

    fn counting_factory(calls: &Arc<AtomicUsize>) -> impl Fn() -> ScopedValue + Send + Sync {
        let calls = Arc::clone(calls);
        move || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            Arc::new(n) as ScopedValue
        }
    }

    fn value_of(value: &ScopedValue) -> Option<usize> {
        value.downcast_ref::<usize>().copied()
    }

    #[tokio::test]
    async fn one_value_per_scope() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory = counting_factory(&calls);
        let key = TypeId::of::<usize>();

        let (first, second) = run_in_new_scope(async {
            let first = resolve(key, "usize", &factory).expect("inside a scope");
            let second = resolve(key, "usize", &factory).expect("inside a scope");
            (first, second)
        })
        .await;
        assert!(Arc::ptr_eq(&first, &second), "one scope, one value");

        let third = run_in_new_scope(async { resolve(key, "usize", &factory) })
            .await
            .expect("inside a scope");
        assert_ne!(
            value_of(&first),
            value_of(&third),
            "a new scope builds anew"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn two_scopes_have_different_ids() {
        let first = ContainerScope::new();
        let second = ContainerScope::new();
        assert_ne!(first.id(), second.id());

        let inside_first = first.run(async { ContainerScope::current_id() }).await;
        let inside_second = second.run(async { ContainerScope::current_id() }).await;
        assert_ne!(inside_first, inside_second, "each task sees its own scope");
    }

    #[tokio::test]
    async fn a_scopes_id_is_stable_and_shared_by_its_handles() {
        let scope = ContainerScope::new();
        let id = scope.id();
        let handle = scope.clone();
        assert_eq!(handle.id(), id, "a clone is the same scope");

        let (first, second, handle_inside) = scope
            .run(async {
                let first = ContainerScope::current_id();
                tokio::task::yield_now().await;
                let second = ContainerScope::current_id();
                let handle_inside = ContainerScope::current().map(|current| current.id());
                (first, second, handle_inside)
            })
            .await;
        assert_eq!(first, Some(id));
        assert_eq!(
            second,
            Some(id),
            "the id does not change while the scope runs"
        );
        assert_eq!(handle_inside, Some(id));
    }

    #[tokio::test]
    async fn a_nested_scope_has_an_id_of_its_own() {
        let (outer, inner, outer_again) = run_in_new_scope(async {
            let outer = ContainerScope::current_id();
            let inner = run_in_new_scope(async { ContainerScope::current_id() }).await;
            (outer, inner, ContainerScope::current_id())
        })
        .await;
        assert!(outer.is_some() && inner.is_some());
        assert_ne!(outer, inner);
        assert_eq!(
            outer, outer_again,
            "the outer id is back once the nested scope ends"
        );
    }

    #[test]
    fn outside_a_scope_there_is_no_current_id() {
        assert_eq!(ContainerScope::current_id(), None);
    }

    #[test]
    fn outside_a_scope_is_an_error_naming_the_type() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory = counting_factory(&calls);
        let error =
            resolve(TypeId::of::<usize>(), "my::Tenant", &factory).expect_err("no scope is active");
        let message = error.to_string();
        assert!(message.contains("my::Tenant"), "{message}");
        assert!(message.contains("scoped binding"), "{message}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "no value is built");
    }

    #[tokio::test]
    async fn a_factory_that_resolves_itself_gets_an_error() {
        let key = TypeId::of::<u8>();
        let inner: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let inner_in = Arc::clone(&inner);
        let factory = move || {
            let noop = || Arc::new(0u8) as ScopedValue;
            let nested = resolve(key, "u8", &noop).err().map(|e| e.to_string());
            *inner_in.lock().expect("unpoisoned") = nested;
            Arc::new(1u8) as ScopedValue
        };

        let value = run_in_new_scope(async { resolve(key, "u8", &factory) })
            .await
            .expect("the outer resolution succeeds");
        assert_eq!(value.downcast_ref::<u8>(), Some(&1));
        let nested = inner.lock().expect("unpoisoned").clone();
        let nested = nested.expect("the nested resolution is an error");
        assert!(nested.contains("dependency cycle"), "{nested}");
    }

    #[tokio::test]
    async fn the_building_mark_is_cleared_after_a_factory_panics() {
        let key = TypeId::of::<u16>();
        run_in_new_scope(async {
            let panicking = || -> ScopedValue { panic!("factory failure") };
            let caught = std::panic::catch_unwind(|| resolve(key, "u16", &panicking));
            assert!(caught.is_err(), "the factory panicked");

            let working = || Arc::new(7u16) as ScopedValue;
            let value = resolve(key, "u16", &working).expect("a retry is not a cycle");
            assert_eq!(value.downcast_ref::<u16>(), Some(&7));
        })
        .await;
    }

    /// Two tasks share a scope. The first builds `A` and its factory
    /// resolves `B`; the second builds `B` and its factory resolves `A`.
    /// The later of the two resolutions closes the cycle and gets the
    /// error, the other waits for a value that then arrives, and neither
    /// thread hangs.
    #[test]
    fn a_cycle_across_two_tasks_is_an_error_for_one_and_neither_hangs() {
        let state = Arc::new(ScopeState::default());
        let first = TypeId::of::<u32>();
        let second = TypeId::of::<u64>();
        // Both builds have claimed their slot before either resolves the
        // other's binding.
        let both_building = Arc::new(Barrier::new(2));
        let (sender, receiver) = mpsc::channel();

        for (own, other) in [(first, second), (second, first)] {
            let state = Arc::clone(&state);
            let both_building = Arc::clone(&both_building);
            let sender = sender.clone();
            thread::spawn(move || {
                let nested_error = Arc::new(Mutex::new(None));
                let nested_error_in = Arc::clone(&nested_error);
                let factory = move || {
                    both_building.wait();
                    let unused = || Arc::new(0u8) as ScopedValue;
                    let nested = resolve(other, "other", &unused).err();
                    *nested_error_in.lock().expect("unpoisoned") = nested.map(|e| e.to_string());
                    Arc::new(1u8) as ScopedValue
                };
                let built = in_scope(&state, || resolve(own, "own", &factory)).is_ok();
                let nested = nested_error.lock().expect("unpoisoned").clone();
                let _ = sender.send((built, nested));
            });
        }

        let results = [bounded(&receiver), bounded(&receiver)];
        assert!(
            results.iter().all(|(built, _)| *built),
            "both builds complete: {results:?}"
        );
        let cycles = results
            .iter()
            .filter(|(_, nested)| {
                nested
                    .as_deref()
                    .is_some_and(|message| message.contains("dependency cycle"))
            })
            .count();
        assert_eq!(cycles, 1, "one resolution closes the cycle: {results:?}");
    }

    /// Two tasks resolve one binding at the same moment. Whichever claims
    /// the slot builds, and its factory waits until the other resolver
    /// waits for it: both get the one value and the factory runs once.
    #[test]
    fn two_tasks_that_resolve_one_binding_at_once_share_one_build() {
        let state = Arc::new(ScopeState::default());
        let key = TypeId::of::<i32>();
        let calls = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::channel();

        for _ in 0..2 {
            let state = Arc::clone(&state);
            let calls = Arc::clone(&calls);
            let sender = sender.clone();
            thread::spawn(move || {
                let factory = {
                    let state = Arc::clone(&state);
                    move || {
                        until_a_resolver_waits(&state);
                        Arc::new(calls.fetch_add(1, Ordering::SeqCst)) as ScopedValue
                    }
                };
                let value = in_scope(&state, || resolve(key, "i32", &factory));
                let _ = sender.send(value.ok().as_ref().and_then(value_of));
            });
        }

        let values = [bounded(&receiver), bounded(&receiver)];
        assert!(values[0].is_some(), "the first resolver gets a value");
        assert_eq!(values[0], values[1], "both resolvers get the one value");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the factory runs once");
    }

    /// A factory that panics leaves the slot empty and wakes the resolver
    /// that waits for it, which then builds the value itself.
    #[test]
    fn a_factory_that_panics_lets_a_waiting_task_build_the_value() {
        let state = Arc::new(ScopeState::default());
        let key = TypeId::of::<i64>();
        // The panicking build has claimed the slot before the other
        // resolver starts.
        let claimed = Arc::new(Barrier::new(2));
        let (panicked_sender, panicked_receiver) = mpsc::channel();
        let (value_sender, value_receiver) = mpsc::channel();

        {
            let state = Arc::clone(&state);
            let claimed = Arc::clone(&claimed);
            thread::spawn(move || {
                let factory = {
                    let state = Arc::clone(&state);
                    move || -> ScopedValue {
                        claimed.wait();
                        until_a_resolver_waits(&state);
                        panic!("factory failure")
                    }
                };
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    in_scope(&state, || resolve(key, "i64", &factory))
                }));
                let _ = panicked_sender.send(outcome.is_err());
            });
        }
        {
            let state = Arc::clone(&state);
            thread::spawn(move || {
                claimed.wait();
                let working = || Arc::new(7i64) as ScopedValue;
                let value = in_scope(&state, || resolve(key, "i64", &working));
                let built = value
                    .ok()
                    .and_then(|value| value.downcast_ref::<i64>().copied());
                let _ = value_sender.send(built);
            });
        }

        assert!(bounded(&panicked_receiver), "the first factory panicked");
        assert_eq!(
            bounded(&value_receiver),
            Some(7),
            "the waiting resolver builds the value"
        );
        let unused = || Arc::new(0i64) as ScopedValue;
        let stored = in_scope(&state, || resolve(key, "i64", &unused)).expect("in the scope");
        assert_eq!(
            stored.downcast_ref::<i64>(),
            Some(&7),
            "the scope keeps the value the second resolver built"
        );
    }
}
