use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use super::Response;

// A sync gate closure, type-erased. Returns a rich `Response` — bool gates are
// wrapped into a bare allow/deny at registration time.
type SyncGateFn = Box<dyn Fn(&dyn Any, &dyn Any) -> Response + Send + Sync>;

// An async gate closure, type-erased.
// The closure returns an owned, boxed future (no borrowed references in the
// output) — this sidesteps lifetime issues with `for<'a>` HRTBs on trait
// objects. Callers clone/copy the user and resource values into the closure.
type AsyncGateFn =
    Box<dyn Fn(&dyn Any, &dyn Any) -> Pin<Box<dyn Future<Output = Response> + Send>> + Send + Sync>;

// A before-hook: receives the (type-erased) user + the action name, returns
// `Some(decision)` to short-circuit the gate or `None` to continue. Keyed by
// the user's `TypeId` (a hook is about the *user's* global privileges, so it is
// resource-agnostic — put resource-specific logic in the gate itself).
type BeforeFn = dyn Fn(&dyn Any, &str) -> Option<bool> + Send + Sync;

// An after-hook: receives the user, the action name, and the running decision
// (`None` while still undecided). Mirrors Laravel's `??=` semantic — an after
// hook can only *fill in* an undecided result, never override an existing one.
// All after hooks still run (so they can log) regardless of the result.
type AfterFn = dyn Fn(&dyn Any, &str, Option<bool>) -> Option<bool> + Send + Sync;

enum GateEntry {
    Sync(SyncGateFn),
    Async(AsyncGateFn),
}

// Downcast the type-erased (user, resource) pair back to the concrete
// `(U, R)` the gate was registered with. Returns `None` when either side
// fails to downcast.
//
// A mismatch is unreachable in normal dispatch: gates are keyed by
// `(action, TypeId::of::<U>(), TypeId::of::<R>())` and only invoked through
// `invoke` / `invoke_async`, which look the entry up by that exact tuple. The
// `None` arm exists so a corrupted dispatch fails **closed** (deny) instead of
// panicking — the request-path panic→500 net would catch a panic, but failing
// closed keeps the authorization posture end-to-end.
fn downcast_pair<'a, U: 'static, R: 'static>(
    u: &'a dyn Any,
    r: &'a dyn Any,
) -> Option<(&'a U, &'a R)> {
    Some((u.downcast_ref::<U>()?, r.downcast_ref::<R>()?))
}

// ── Default denial response ────────────────────────────────────────────────
//
// Process-global, like the `before`/`after` hooks conceptually are — but
// unlike those (keyed by the user's `TypeId`, so a hook only ever applies to
// its own user type), this has no key to scope by: it is Laravel's
// `Gate::$defaultDenialResponse` static, consulted only where a *bare* bool
// result collapses into a `Response` (see `bool_to_response` below) and
// where `Gate::inspect`/`inspect_async` normalize an undecided evaluation.
// Set once via `Gate::default_denial_response`, typically in
// `bootstrap::register()`.
static DEFAULT_DENIAL: RwLock<Option<Response>> = RwLock::new(None);

/// The current default denial response: the bare `Response::deny()` until
/// [`crate::Gate::default_denial_response`] sets something else.
///
/// Poison degrades to the bare deny (logged), matching every other lock in
/// this registry — a poisoned lock never aborts the process, it safe-denies.
pub(crate) fn default_denial() -> Response {
    match DEFAULT_DENIAL.read() {
        Ok(guard) => guard.clone().unwrap_or_else(Response::deny),
        Err(_) => {
            tracing::error!(
                "Default denial response lock poisoned during read; \
                 falling back to a bare deny."
            );
            Response::deny()
        }
    }
}

/// Set the process-global default denial response. See
/// [`crate::Gate::default_denial_response`].
///
/// An allow-shaped `response` is rejected: this is a *denial* default, and
/// accepting `Response::allow()` here would silently invert every bare
/// `false` bool-gate result to allowed — the one fail-open direction on this
/// surface. Laravel has no equivalent guard; Suprnova adds one deliberately.
pub(crate) fn set_default_denial(response: Response) {
    if response.allowed() {
        tracing::error!(
            "Gate::default_denial_response was given an allow-shaped Response; \
             ignoring it and keeping the previous default. A denial default must \
             deny — an allow-shaped one would silently invert every bare `false` \
             gate result to allowed."
        );
        return;
    }
    match DEFAULT_DENIAL.write() {
        Ok(mut guard) => *guard = Some(response),
        Err(_) => {
            tracing::error!("Default denial response lock poisoned during write; skipping set.")
        }
    }
}

/// Reset the default denial response to unset (bare deny). Backs
/// [`crate::Gate::clear_default_denial_response`] — hermetic tests need this
/// so a default one test sets cannot leak into the next.
pub(crate) fn clear_default_denial() {
    match DEFAULT_DENIAL.write() {
        Ok(mut guard) => *guard = None,
        Err(_) => tracing::error!("Default denial response lock poisoned during clear; skipping."),
    }
}

// Convert a bare bool gate/hook result into a `Response`. This is the single
// seam every bool-returning path (`register`, `register_async`, the
// before/after hooks below) uses to reach a `Response` — a gate registered
// with `register_with`/`register_async_with` never calls this, because its
// closure already returns the `Response` it wants. That split is the whole
// point of `Gate::default_denial_response`: it reshapes a bare `false`, and
// only a bare `false` — an explicit `Response::deny_with(...)` (or any other
// rich denial) a `define_with` gate returns always passes through verbatim.
fn bool_to_response(allowed: bool) -> Response {
    if allowed {
        Response::allow()
    } else {
        default_denial()
    }
}

pub(crate) struct GateRegistry {
    gates: RwLock<HashMap<(String, TypeId, TypeId), GateEntry>>,
    // before/after hooks are stored behind `Arc` so the evaluation path can
    // clone the hook list out under a short read lock and invoke the user
    // closures *outside* the lock — a before hook that itself calls
    // `Gate::allows` re-enters this registry, and holding the read lock across
    // that nested call could deadlock a non-reentrant `RwLock`.
    before: RwLock<HashMap<TypeId, Vec<Arc<BeforeFn>>>>,
    after: RwLock<HashMap<TypeId, Vec<Arc<AfterFn>>>>,
}

impl GateRegistry {
    pub(crate) fn new() -> Self {
        Self {
            gates: RwLock::new(HashMap::new()),
            before: RwLock::new(HashMap::new()),
            after: RwLock::new(HashMap::new()),
        }
    }

    // ── Gate registration ────────────────────────────────────────────────────

    pub(crate) fn register<U: 'static, R: 'static>(
        &self,
        action: &str,
        f: impl Fn(&U, &R) -> bool + Send + Sync + 'static,
    ) {
        let erased: SyncGateFn = Box::new(move |u, r| match downcast_pair::<U, R>(u, r) {
            Some((u, r)) => bool_to_response(f(u, r)),
            None => Response::deny(),
        });
        self.insert_gate::<U, R>(action, GateEntry::Sync(erased), "sync");
    }

    /// Register a sync gate whose closure returns a rich [`Response`] directly
    /// (so denials can carry a message / status). See [`register`](Self::register)
    /// for the bool form.
    pub(crate) fn register_with<U: 'static, R: 'static>(
        &self,
        action: &str,
        f: impl Fn(&U, &R) -> Response + Send + Sync + 'static,
    ) {
        let erased: SyncGateFn = Box::new(move |u, r| match downcast_pair::<U, R>(u, r) {
            Some((u, r)) => f(u, r),
            None => Response::deny(),
        });
        self.insert_gate::<U, R>(action, GateEntry::Sync(erased), "sync(response)");
    }

    /// Register an async gate.
    ///
    /// The closure must produce an *owned* future — it cannot borrow `user` or
    /// `resource` because the type-erased `&dyn Any` references cannot outlive
    /// the `GateRegistry`. Callers are expected to clone / copy any data they
    /// need inside the closure body before returning the future.
    pub(crate) fn register_async<U, R, F, Fut>(&self, action: &str, f: F)
    where
        U: 'static,
        R: 'static,
        F: Fn(&U, &R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        let erased: AsyncGateFn = Box::new(move |u, r| {
            let out: Pin<Box<dyn Future<Output = Response> + Send>> =
                match downcast_pair::<U, R>(u, r) {
                    Some((u, r)) => {
                        let fut = f(u, r);
                        Box::pin(async move { bool_to_response(fut.await) })
                    }
                    None => Box::pin(async { Response::deny() }),
                };
            out
        });
        self.insert_gate::<U, R>(action, GateEntry::Async(erased), "async");
    }

    /// Register an async gate whose future resolves to a rich [`Response`].
    pub(crate) fn register_async_with<U, R, F, Fut>(&self, action: &str, f: F)
    where
        U: 'static,
        R: 'static,
        F: Fn(&U, &R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let erased: AsyncGateFn = Box::new(move |u, r| {
            let out: Pin<Box<dyn Future<Output = Response> + Send>> =
                match downcast_pair::<U, R>(u, r) {
                    Some((u, r)) => Box::pin(f(u, r)),
                    None => Box::pin(async { Response::deny() }),
                };
            out
        });
        self.insert_gate::<U, R>(action, GateEntry::Async(erased), "async(response)");
    }

    // Insert a gate entry, degrading gracefully on lock poison rather than
    // panicking the boot path. Skipping a single registration is recoverable:
    // the gate's authorize calls return None → safe-deny.
    //
    // Last-writer-wins on duplicate `(action, U, R)` matches Laravel's
    // `Gate::define` semantic (PHP overwrites the abilities map entry), so
    // re-registering an ability replaces the previous closure. That's the
    // right default — tests, hot reload, and intentional overrides all rely
    // on it. But silent replacement in a process-global registry makes
    // accidental duplicates (two policies, two bootstrap closures, an
    // inventory + an explicit call) impossible to audit. Emit a
    // `tracing::warn!` whenever a registration overwrites an existing entry
    // so the conflict is visible in logs without breaking the semantic.
    fn insert_gate<U: 'static, R: 'static>(&self, action: &str, entry: GateEntry, kind: &str) {
        let key = (action.to_string(), TypeId::of::<U>(), TypeId::of::<R>());
        match self.gates.write() {
            Ok(mut gates) => {
                let previous_kind = gates.get(&key).map(|e| match e {
                    GateEntry::Sync(_) => "sync",
                    GateEntry::Async(_) => "async",
                });
                if let Some(previous_kind) = previous_kind {
                    tracing::warn!(
                        action = %action,
                        previous_kind = %previous_kind,
                        new_kind = %kind,
                        user_type = std::any::type_name::<U>(),
                        resource_type = std::any::type_name::<R>(),
                        "Gate registration is overwriting an existing gate for \
                         (action, user, resource); the previous closure will no \
                         longer be invoked. This matches Laravel's last-writer-wins \
                         semantic but is reported so duplicate registrations from \
                         policies, inventory, or bootstrap code are auditable."
                    );
                }
                gates.insert(key, entry);
            }
            Err(_) => {
                tracing::error!(
                    action = %action,
                    kind = %kind,
                    user_type = std::any::type_name::<U>(),
                    resource_type = std::any::type_name::<R>(),
                    "Gate registry lock poisoned; skipping gate registration. \
                     Calls to Gate::authorize for this action will safe-deny."
                );
            }
        }
    }

    // ── before / after hooks ───────────────────────────────────────────────

    /// Register a before-hook keyed by the user type `U`. Runs before any gate
    /// for an `allows`/`inspect` on that user type; first `Some` short-circuits.
    pub(crate) fn register_before<U: 'static>(
        &self,
        f: impl Fn(&U, &str) -> Option<bool> + Send + Sync + 'static,
    ) {
        let erased: Arc<BeforeFn> = Arc::new(move |u: &dyn Any, action: &str| {
            // Type-erased downcast mismatch is unreachable in normal dispatch
            // (hooks are keyed by `TypeId::of::<U>()` and only invoked for that
            // user type). Fail safe by abstaining — `None` lets evaluation
            // continue to the gate, which defaults to deny.
            match u.downcast_ref::<U>() {
                Some(u) => f(u, action),
                None => None,
            }
        });
        match self.before.write() {
            Ok(mut map) => map.entry(TypeId::of::<U>()).or_default().push(erased),
            Err(_) => tracing::error!(
                user_type = std::any::type_name::<U>(),
                "before-hook registry poisoned; skipping registration."
            ),
        }
    }

    /// Register an after-hook keyed by the user type `U`. Runs after the gate
    /// on an `allows`/`inspect`; can only fill an undecided result.
    pub(crate) fn register_after<U: 'static>(
        &self,
        f: impl Fn(&U, &str, Option<bool>) -> Option<bool> + Send + Sync + 'static,
    ) {
        let erased: Arc<AfterFn> =
            Arc::new(move |u: &dyn Any, action: &str, current: Option<bool>| {
                // Type-erased downcast mismatch is unreachable in normal
                // dispatch (hooks are keyed by `TypeId::of::<U>()`). Fail
                // safe by abstaining — an after hook can only *fill* an
                // undecided result, so `None` leaves the decision untouched.
                match u.downcast_ref::<U>() {
                    Some(u) => f(u, action, current),
                    None => None,
                }
            });
        match self.after.write() {
            Ok(mut map) => map.entry(TypeId::of::<U>()).or_default().push(erased),
            Err(_) => tracing::error!(
                user_type = std::any::type_name::<U>(),
                "after-hook registry poisoned; skipping registration."
            ),
        }
    }

    // Clone the hook list for a user type out from under a short read lock so
    // the closures can be invoked without holding the lock (see `before`/`after`
    // field docs for the re-entrancy reasoning). Poison → empty (safe-skip).
    fn before_hooks(&self, tid: TypeId) -> Vec<Arc<BeforeFn>> {
        match self.before.read() {
            Ok(map) => map.get(&tid).cloned().unwrap_or_default(),
            Err(_) => {
                tracing::error!("before-hook registry poisoned during read; skipping hooks.");
                Vec::new()
            }
        }
    }

    fn after_hooks(&self, tid: TypeId) -> Vec<Arc<AfterFn>> {
        match self.after.read() {
            Ok(map) => map.get(&tid).cloned().unwrap_or_default(),
            Err(_) => {
                tracing::error!("after-hook registry poisoned during read; skipping hooks.");
                Vec::new()
            }
        }
    }

    // ── Gate invocation ──────────────────────────────────────────────────────

    /// Invoke a sync gate. Returns `None` if no gate is registered for the
    /// `(action, U, R)` tuple, `Some(deny)` if the gate is registered as async
    /// (caller must use `invoke_async`).
    ///
    /// Hitting the async-registered branch via the sync path is almost always
    /// a caller bug — silently denying would hide it, so we emit a
    /// `tracing::warn!` and the caller can spot it in logs.
    pub(crate) fn invoke<U: 'static, R: 'static>(
        &self,
        action: &str,
        user: &U,
        resource: &R,
    ) -> Option<Response> {
        let key = (action.to_string(), TypeId::of::<U>(), TypeId::of::<R>());
        let gates = match self.gates.read() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!(
                    action = %action,
                    user_type = std::any::type_name::<U>(),
                    resource_type = std::any::type_name::<R>(),
                    "Gate registry lock poisoned during invoke; safe-denying \
                     (returning None so inspect/authorize apply the configured \
                     default denial response, or Unauthorized when none is set)."
                );
                return None;
            }
        };
        match gates.get(&key) {
            Some(GateEntry::Sync(f)) => Some(f(user as &dyn Any, resource as &dyn Any)),
            Some(GateEntry::Async(_)) => {
                tracing::warn!(
                    action = %action,
                    user_type = std::any::type_name::<U>(),
                    resource_type = std::any::type_name::<R>(),
                    "Gate::allows/denies/authorize called on an async-registered gate — \
                     defaulting to deny. Use Gate::allows_async/denies_async/authorize_async instead.",
                );
                // Route through the configured default so this caller-bug
                // safety net doesn't leak a bare 403 past a `deny_as_not_found`
                // default meant to hide the resource's existence uniformly.
                Some(default_denial())
            }
            None => None,
        }
    }

    /// Invoke a gate asynchronously. Works for both sync- and async-registered
    /// gates. Returns `None` if the gate is not registered.
    pub(crate) async fn invoke_async<U: 'static, R: 'static>(
        &self,
        action: &str,
        user: &U,
        resource: &R,
    ) -> Option<Response> {
        type AsyncFut = Pin<Box<dyn Future<Output = Response> + Send>>;
        type EntryResult = Option<Result<Response, AsyncFut>>;

        let key = (action.to_string(), TypeId::of::<U>(), TypeId::of::<R>());
        // We hold the read lock only long enough to clone the result or start
        // the async dispatch — we must NOT hold it across an `.await`.
        //
        // Degrade to None on poison; `Gate::inspect_async`/`authorize_async`
        // apply the configured default denial response (or Unauthorized when
        // none is set) — see `default_denial` above.
        let entry_result: EntryResult = match self.gates.read() {
            Ok(gates) => match gates.get(&key) {
                Some(GateEntry::Sync(f)) => Some(Ok(f(user as &dyn Any, resource as &dyn Any))),
                Some(GateEntry::Async(f)) => Some(Err(f(user as &dyn Any, resource as &dyn Any))),
                None => None,
            },
            Err(_) => {
                tracing::error!(
                    action = %action,
                    user_type = std::any::type_name::<U>(),
                    resource_type = std::any::type_name::<R>(),
                    "Gate registry lock poisoned during invoke_async; safe-denying."
                );
                None
            }
        };

        match entry_result {
            None => None,
            Some(Ok(result)) => Some(result),
            Some(Err(fut)) => Some(fut.await),
        }
    }

    // ── Full evaluation: before → gate → after ──────────────────────────────

    /// Evaluate `(action, U, R)` through the full Laravel pipeline: before
    /// hooks (first `Some` wins), then the gate, then after hooks (fill-only).
    /// Returns `None` when nothing decided (no before hook, no gate, no after
    /// hook) — callers normalize that to a default deny.
    pub(crate) fn raw<U: 'static, R: 'static>(
        &self,
        action: &str,
        user: &U,
        resource: &R,
    ) -> Option<Response> {
        let tid = TypeId::of::<U>();
        let mut result = self.run_before(tid, user as &dyn Any, action);
        if result.is_none() {
            result = self.invoke::<U, R>(action, user, resource);
        }
        self.run_after(tid, user as &dyn Any, action, result)
    }

    /// Async sibling of [`raw`](Self::raw). before/after hooks are synchronous;
    /// only the gate dispatch awaits.
    pub(crate) async fn raw_async<U: 'static, R: 'static>(
        &self,
        action: &str,
        user: &U,
        resource: &R,
    ) -> Option<Response> {
        let tid = TypeId::of::<U>();
        let mut result = self.run_before(tid, user as &dyn Any, action);
        if result.is_none() {
            result = self.invoke_async::<U, R>(action, user, resource).await;
        }
        self.run_after(tid, user as &dyn Any, action, result)
    }

    // Run before hooks; first `Some` short-circuits.
    fn run_before(&self, tid: TypeId, user: &dyn Any, action: &str) -> Option<Response> {
        for hook in self.before_hooks(tid) {
            if let Some(decision) = hook(user, action) {
                return Some(bool_to_response(decision));
            }
        }
        None
    }

    // Run all after hooks (so they can log), filling the result only while it
    // is still undecided — Laravel's `$result ??= $afterResult`.
    fn run_after(
        &self,
        tid: TypeId,
        user: &dyn Any,
        action: &str,
        mut result: Option<Response>,
    ) -> Option<Response> {
        for hook in self.after_hooks(tid) {
            let current = result.as_ref().map(Response::allowed);
            let filled = hook(user, action, current);
            // Fill-only: an after hook can decide an undecided result, never
            // override one already produced by a before hook or gate.
            if result.is_none() {
                result = filled.map(bool_to_response);
            }
        }
        result
    }

    // ── Introspection ────────────────────────────────────────────────────────

    /// Whether a gate (sync or async) is registered for `(action, U, R)`. Used
    /// by [`crate::Gate::has`] for the introspection path Laravel calls
    /// `Gate::has`.
    pub(crate) fn has<U: 'static, R: 'static>(&self, action: &str) -> bool {
        let key = (action.to_string(), TypeId::of::<U>(), TypeId::of::<R>());
        match self.gates.read() {
            Ok(gates) => gates.contains_key(&key),
            // Poisoned lock: same safe-deny posture as the invoke
            // paths — pretend the gate is absent rather than panic.
            Err(_) => false,
        }
    }

    /// Distinct action names across every registered (action, U, R)
    /// tuple. Used by [`crate::Gate::abilities`] — mirrors Laravel's
    /// `Gate::abilities()` (which also dedupes by action name).
    /// Returns an empty vec on lock poison (same safe-deny shape).
    pub(crate) fn abilities(&self) -> Vec<String> {
        let Ok(gates) = self.gates.read() else {
            tracing::error!(
                "Gate registry lock poisoned during abilities(); returning empty list."
            );
            return Vec::new();
        };
        let mut names: Vec<String> = gates.keys().map(|(a, _, _)| a.clone()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

pub(crate) fn global() -> &'static GateRegistry {
    static R: std::sync::OnceLock<GateRegistry> = std::sync::OnceLock::new();
    R.get_or_init(GateRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct U;
    struct R;

    #[tracing_test::traced_test]
    #[test]
    fn re_registering_overwrites_with_last_writer_and_warns() {
        // Each test owns a fresh registry — the production global is shared
        // across the process, and we don't want bleed-over from other tests
        // that may also register a gate named "duplicate-probe".
        let registry = GateRegistry::new();

        registry.register::<U, R>("duplicate-probe", |_u, _r| true);
        registry.register::<U, R>("duplicate-probe", |_u, _r| false);

        let decision = registry
            .invoke::<U, R>("duplicate-probe", &U, &R)
            .expect("gate is registered");
        assert!(
            !decision.allowed(),
            "last-writer-wins: the second registration must replace the first"
        );

        assert!(
            logs_contain("overwriting an existing gate"),
            "duplicate registration must emit a warn so it is auditable"
        );
    }

    #[tracing_test::traced_test]
    #[test]
    fn first_registration_does_not_warn() {
        let registry = GateRegistry::new();

        registry.register::<U, R>("fresh-probe", |_u, _r| true);

        assert!(
            !logs_contain("overwriting an existing gate"),
            "a first-time registration must not warn"
        );
    }

    #[test]
    fn downcast_pair_returns_none_on_type_mismatch() {
        // The type-erased gate closures call `downcast_pair` to recover the
        // concrete `(U, R)` they were registered with. A mismatch must yield
        // `None` (so the gate fails closed) rather than panicking.
        let u = U;
        let r = R;
        let bad = "wrong type";

        // Correct types round-trip.
        assert!(downcast_pair::<U, R>(&u as &dyn Any, &r as &dyn Any).is_some());
        // A mismatched user side fails closed.
        assert!(downcast_pair::<U, R>(&bad as &dyn Any, &r as &dyn Any).is_none());
        // A mismatched resource side fails closed.
        assert!(downcast_pair::<U, R>(&u as &dyn Any, &bad as &dyn Any).is_none());
    }

    #[test]
    fn type_erased_gate_denies_on_downcast_mismatch_without_panicking() {
        // Build the registered sync closure exactly as `register` does, then
        // invoke it with a wrong-typed user. Before the fix this `.expect()`ed
        // and panicked; now it must deny (fail closed) and return without
        // unwinding.
        let erased: SyncGateFn = Box::new(move |u, r| match downcast_pair::<U, R>(u, r) {
            Some((_u, _r)) => Response::from(true),
            None => Response::deny(),
        });

        let wrong_user = "not a U";
        let r = R;
        let decision = erased(&wrong_user as &dyn Any, &r as &dyn Any);
        assert!(
            !decision.allowed(),
            "a downcast type mismatch must fail closed (deny), not panic"
        );
    }

    #[tracing_test::traced_test]
    #[test]
    fn sync_overwriting_async_warns_with_kind_transition() {
        let registry = GateRegistry::new();

        registry.register_async::<U, R, _, _>("kind-flip", |_u, _r| async { true });
        registry.register::<U, R>("kind-flip", |_u, _r| false);

        assert!(
            logs_contain("overwriting an existing gate"),
            "a sync registration replacing an async one must still warn"
        );
        assert!(
            logs_contain("previous_kind") && logs_contain("async"),
            "the warn must record the previous gate kind for diagnostics"
        );
    }
}
