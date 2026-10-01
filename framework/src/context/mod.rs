//! Per-request key/value bag scoped via `tokio::task_local!`.
//!
//! Laravel-shaped: `Context::add(key, val)` for visible storage,
//! `Context::hidden_add(key, val)` for storage that doesn't appear in
//! `Context::all()` (sensitive data you want available to deep callers
//! but not serialized into logs). `Context::push(key, val)` appends to
//! a stack at that key. `Context::forget(key)` removes.
//!
//! Operations outside an active scope are silent no-ops - early-boot
//! code, tests without middleware setup, and background tasks that
//! choose not to install a scope all keep working without panics.
//! When a mutation falls on no scope it emits a `tracing::trace!`
//! event on the `suprnova::context` target so misordered middleware
//! and missing-propagation bugs are observable in instrumented runs
//! without changing the no-panic contract.
//!
//! ### Propagating into spawned tasks
//!
//! Tokio task-locals do not flow through [`tokio::spawn`]: a child
//! task starts with an empty `CONTEXT` and `Context::get` returns
//! `None`. Use [`Context::current`] to snapshot the live store and
//! [`Context::scope`] to re-enter it inside the child:
//!
//! ```rust,no_run
//! # async fn ex() {
//! if let Some(store) = suprnova::context::Context::current() {
//!     tokio::spawn(suprnova::context::Context::scope(store, async move {
//!         // `Context::get`, `query_param`, etc. now see the parent bag.
//!     }));
//! }
//! # }
//! ```
//!
//! [`ContextStore`] holds `Arc<DashMap>` handles, so the propagated
//! store is *live-shared* with the parent - writes from either side
//! are visible to the other for as long as the child holds the clone.
//! This is what audit/logging spawns want; if you need an isolated
//! snapshot, clone the maps explicitly.
//!
//! ### Travelling with queued work
//!
//! A queued job, a queued mail or notification, and a queued event listener
//! run after the request that asked for them, often in another process. The
//! framework carries the context to them: a push takes a
//! [`ContextSnapshot`] of the visible and hidden bags
//! ([`Context::dehydrate`]), the envelope stores it, and the worker runs the
//! job inside a scope restored from it ([`Context::restored`]). A value a
//! request adds, such as a trace id or a tenant, is there when the job reads
//! it. The job works on its own copy, so what it adds does not reach the
//! request, or the next job.
//!
//! The query bag does not travel. It describes the request that was being
//! served, and the work that runs later is not that request.
//!
//! Code that serves a request always has context to carry: the request
//! middleware adds the request's id as `_request_id`, so that a job, and
//! the log lines it writes, can be traced to the request that queued it.
//! Code outside a request, such as a console command or a scheduled task,
//! carries only what it added itself.
//!
//! [`Context::dehydrating`] and [`Context::hydrated`] register callbacks for
//! the two ends of the trip, as Laravel's hooks of the same names do.

use dashmap::DashMap;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

/// The backing store inside a request's context scope. Two maps -
/// visible (`data`) and hidden (`hidden`) - so logging serializers
/// can dump `all()` without leaking secrets.
///
/// `query` is the request's query-parameter snapshot. Populated by the
/// request middleware from the URL query string at scope-entry; read
/// by [`Context::query_param`] downstream. Stored separately from
/// `data` so paginate / scope-aware code can't accidentally collide
/// with user-set context keys.
///
/// The `Debug` output names the hidden keys and never their values, for the
/// reason the hidden bag exists.
#[derive(Default, Clone)]
pub struct ContextStore {
    data: Arc<DashMap<String, Value>>,
    hidden: Arc<DashMap<String, Value>>,
    query: Arc<DashMap<String, String>>,
}

impl std::fmt::Debug for ContextStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut hidden_keys: Vec<String> = self.hidden.iter().map(|kv| kv.key().clone()).collect();
        hidden_keys.sort();
        f.debug_struct("ContextStore")
            .field("data", &self.data)
            .field("hidden_keys", &hidden_keys)
            .field("query", &self.query)
            .finish()
    }
}

impl ContextStore {
    /// Construct a store pre-populated with the supplied query map.
    /// Used by the request middleware so `Context::query_param` reads
    /// the real request's `?key=value` pairs.
    pub fn with_query(query: HashMap<String, String>) -> Self {
        let q = DashMap::with_capacity(query.len());
        for (k, v) in query {
            q.insert(k, v);
        }
        Self {
            data: Arc::new(DashMap::new()),
            hidden: Arc::new(DashMap::new()),
            query: Arc::new(q),
        }
    }
}

impl ContextStore {
    /// A fresh store holding a snapshot's two bags, with an empty query
    /// bag. The store shares nothing with the snapshot: a write to it
    /// changes neither the snapshot nor the context it was taken from.
    pub fn from_snapshot(snapshot: &ContextSnapshot) -> Self {
        let fill = |bag: &BTreeMap<String, Value>| {
            let map = DashMap::with_capacity(bag.len());
            for (key, value) in bag {
                map.insert(key.clone(), value.clone());
            }
            Arc::new(map)
        };
        Self {
            data: fill(&snapshot.data),
            hidden: fill(&snapshot.hidden),
            query: Arc::new(DashMap::new()),
        }
    }
}

/// The part of a context that travels with queued work: the visible and the
/// hidden bag, as plain maps.
///
/// It is what a queue envelope stores, so it is `Serialize` and
/// `Deserialize`, and an empty bag stays off the wire. Its `Debug` output
/// names the hidden keys and never their values, because a hidden value is
/// one the application asked to keep out of logs.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// The visible bag, what [`Context::all`] returns.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, Value>,
    /// The hidden bag, what [`Context::hidden_get`] reads.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden: BTreeMap<String, Value>,
}

impl ContextSnapshot {
    /// `true` when both bags are empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty() && self.hidden.is_empty()
    }

    /// A copy without the hidden bag, for output that more people can read
    /// than the queue store, such as a log line.
    pub fn without_hidden(&self) -> Self {
        Self {
            data: self.data.clone(),
            hidden: BTreeMap::new(),
        }
    }
}

impl std::fmt::Debug for ContextSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextSnapshot")
            .field("data", &self.data)
            .field("hidden_keys", &self.hidden.keys().collect::<Vec<_>>())
            .finish()
    }
}

type DehydratingHook = Arc<dyn Fn(&mut ContextSnapshot) + Send + Sync>;
type HydratedHook = Arc<dyn Fn(&ContextSnapshot) + Send + Sync>;

static DEHYDRATING_HOOKS: RwLock<Vec<DehydratingHook>> = RwLock::new(Vec::new());
static HYDRATED_HOOKS: RwLock<Vec<HydratedHook>> = RwLock::new(Vec::new());

/// The registered hooks, cloned out so none runs under the registry lock. A
/// poisoned registry yields the hooks it holds: a panic in one registration
/// must not stop context from travelling.
fn hooks<T: Clone>(registry: &RwLock<Vec<T>>) -> Vec<T> {
    registry
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

tokio::task_local! {
    /// Per-request task-local context store. Installed by the request
    /// pipeline; readers reach it via [`Context`](crate::context::Context).
    pub static CONTEXT: ContextStore;
}

// Testing override for `Context::query_param`.
//
// Per-thread so parallel tests don't collide - `#[tokio::test]` uses a
// current-thread runtime by default, so the future is driven on the
// calling OS thread and `thread_local!` isolates each test.
//
// Tests outside a `CONTEXT.scope` (the common case for unit tests of
// pure-function paginate logic) can install query params via
// `Context::test_set_query` without paying the cost of wrapping every
// async block in a context scope. Reads from the override take
// priority over the scoped `CONTEXT.query` bag.
//
// Only the testing hooks mutate this slot. Those hooks compile under
// `cfg(test)` or the default-enabled `testing` Cargo feature; consumers can
// remove them with `default-features = false` while the reader remains
// compiled so the fast path stays uniform.
thread_local! {
    static QUERY_OVERRIDE: RefCell<Option<HashMap<String, String>>> =
        const { RefCell::new(None) };
}

/// Facade for the per-request key/value bag.
pub struct Context;

impl Context {
    /// Snapshot the active context store, or `None` if no scope is
    /// installed on the current task.
    ///
    /// The returned [`ContextStore`] shares the parent's underlying
    /// maps via `Arc`, so it can be moved into a [`tokio::spawn`]ed
    /// task and re-entered via [`Self::scope`] to give the child task
    /// access to the same `data` / `hidden` / `query` bags. See the
    /// module-level docs for the propagation pattern.
    pub fn current() -> Option<ContextStore> {
        CONTEXT.try_with(|store| store.clone()).ok()
    }

    /// Enter `store` as the active context for the duration of `fut`.
    ///
    /// Thin wrapper around `CONTEXT.scope(store, fut)` so callers can
    /// hand the spawned future to [`tokio::spawn`] without naming the
    /// task-local directly:
    ///
    /// ```rust,no_run
    /// # use suprnova::context::Context;
    /// # async fn ex() {
    /// if let Some(store) = Context::current() {
    ///     tokio::spawn(Context::scope(store, async move { /* ... */ }));
    /// }
    /// # }
    /// ```
    pub fn scope<F>(
        store: ContextStore,
        fut: F,
    ) -> tokio::task::futures::TaskLocalFuture<ContextStore, F>
    where
        F: std::future::Future,
    {
        CONTEXT.scope(store, fut)
    }

    /// Snapshot the context for work that runs later: the visible and the
    /// hidden bag, after every [`Self::dehydrating`] callback has seen the
    /// snapshot.
    ///
    /// Returns `None` when there is nothing to carry: outside a scope with
    /// no callback adding a value, or when both bags are empty. A push then
    /// writes an envelope with no context on it. Inside a request there is
    /// always the request's id to carry, see the module docs.
    pub fn dehydrate() -> Option<ContextSnapshot> {
        let mut snapshot = CONTEXT
            .try_with(|store| {
                let copy = |bag: &DashMap<String, Value>| {
                    bag.iter()
                        .map(|kv| (kv.key().clone(), kv.value().clone()))
                        .collect::<BTreeMap<_, _>>()
                };
                ContextSnapshot {
                    data: copy(&store.data),
                    hidden: copy(&store.hidden),
                }
            })
            .unwrap_or_default();
        for hook in hooks(&DEHYDRATING_HOOKS) {
            hook(&mut snapshot);
        }
        (!snapshot.is_empty()).then_some(snapshot)
    }

    /// Build the store that queued work runs in: a fresh one that holds
    /// `snapshot`'s two bags, or an empty one for `None`.
    ///
    /// Every [`Self::hydrated`] callback runs before this returns, inside
    /// the new store's scope, when there is a snapshot to read. Enter the
    /// store with [`Self::scope`]. A queue worker enters the same store for
    /// the job and for the lifecycle events around it, so a listener of
    /// `JobProcessed` reads what the job read and what the job added.
    pub fn hydrate(snapshot: Option<&ContextSnapshot>) -> ContextStore {
        let Some(snapshot) = snapshot else {
            return ContextStore::default();
        };
        let store = ContextStore::from_snapshot(snapshot);
        let hydrated = hooks(&HYDRATED_HOOKS);
        if !hydrated.is_empty() {
            CONTEXT.sync_scope(store.clone(), || {
                for hook in hydrated {
                    hook(snapshot);
                }
            });
        }
        store
    }

    /// Run `fut` inside a fresh scope that holds `snapshot`'s two bags:
    /// [`Self::hydrate`], then [`Self::scope`].
    pub async fn restored<F>(snapshot: Option<ContextSnapshot>, fut: F) -> F::Output
    where
        F: std::future::Future,
    {
        CONTEXT.scope(Self::hydrate(snapshot.as_ref()), fut).await
    }

    /// Register a callback that runs on every snapshot taken for queued
    /// work, before it is stored. Mirrors Laravel's `Context::dehydrating`.
    ///
    /// The callback may add, change or remove entries. Use it to carry a
    /// value that lives outside the context, such as the request's locale,
    /// or to keep a value from leaving the process. It changes the
    /// snapshot only, never the live context. Register it once, at boot.
    pub fn dehydrating(hook: impl Fn(&mut ContextSnapshot) + Send + Sync + 'static) {
        DEHYDRATING_HOOKS
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::new(hook));
    }

    /// Register a callback that runs when a worker has restored a snapshot,
    /// before the job runs. Mirrors Laravel's `Context::hydrated`.
    ///
    /// It runs inside the restored scope, so the `Context` methods read and
    /// write the job's context. Use it to put a carried value back where
    /// the application reads it. Register it once, at boot. It runs once
    /// for every attempt of a job, because every attempt starts from the
    /// snapshot.
    pub fn hydrated(hook: impl Fn(&ContextSnapshot) + Send + Sync + 'static) {
        HYDRATED_HOOKS
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::new(hook));
    }

    /// **Testing hook.** Remove every [`Self::dehydrating`] and
    /// [`Self::hydrated`] callback. The callbacks are process-wide, so a
    /// test that registers one removes it again to stay hermetic.
    ///
    /// Compiled under `cfg(test)` or the `testing` Cargo feature. `testing` is
    /// enabled by default, including in release builds; consumers can remove
    /// this hook with `default-features = false` and without `testing`.
    #[cfg(any(test, feature = "testing"))]
    pub fn test_clear_hooks() {
        DEHYDRATING_HOOKS
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        HYDRATED_HOOKS
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// Set `key` to `value` (replacing any existing entry).
    pub fn add<K, V>(key: K, value: V)
    where
        K: Into<String>,
        V: Serialize,
    {
        if CONTEXT
            .try_with(|store| {
                let key = key.into();
                match serde_json::to_value(value) {
                    Ok(v) => {
                        store.data.insert(key, v);
                    }
                    Err(err) => {
                        tracing::trace!(
                            target: "suprnova::context",
                            op = "add",
                            key = ?key,
                            error = %err,
                            "Context mutation discarded: value failed to serialize",
                        );
                    }
                }
            })
            .is_err()
        {
            tracing::trace!(
                target: "suprnova::context",
                op = "add",
                "Context mutation discarded: no active scope on this task",
            );
        }
    }

    /// Read `key` and deserialize. Returns `None` if absent, outside a
    /// scope, or if the stored value isn't of type `T`.
    ///
    /// A wrong-type read (`key` present but `serde_json::from_value::<T>`
    /// errors) emits a `tracing::trace!` on the `suprnova::context`
    /// target so the bug is observable in instrumented runs; plain
    /// absence stays silent so logs aren't flooded by intentional
    /// "is this set?" probes.
    pub fn get<T: DeserializeOwned>(key: &str) -> Option<T> {
        CONTEXT
            .try_with(|store| {
                let raw = store.data.get(key)?;
                match serde_json::from_value::<T>(raw.value().clone()) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        tracing::trace!(
                            target: "suprnova::context",
                            op = "get",
                            key = ?key,
                            expected = std::any::type_name::<T>(),
                            error = %err,
                            "Context read returned None: value present but did not deserialize",
                        );
                        None
                    }
                }
            })
            .ok()
            .flatten()
    }

    /// Push `value` onto a stack stored at `key`. Initializes an empty
    /// vec on the first push; converts a scalar at `key` into a
    /// `[scalar, value]` array on subsequent push.
    pub fn push<K, V>(key: K, value: V)
    where
        K: Into<String>,
        V: Serialize,
    {
        if CONTEXT
            .try_with(|store| {
                let key = key.into();
                let new_val = match serde_json::to_value(value) {
                    Ok(v) => v,
                    Err(err) => {
                        tracing::trace!(
                            target: "suprnova::context",
                            op = "push",
                            key = ?key,
                            error = %err,
                            "Context mutation discarded: value failed to serialize",
                        );
                        return;
                    }
                };
                store
                    .data
                    .entry(key)
                    .and_modify(|existing| {
                        if let Value::Array(arr) = existing {
                            arr.push(new_val.clone());
                        } else {
                            *existing = Value::Array(vec![existing.clone(), new_val.clone()]);
                        }
                    })
                    .or_insert_with(|| Value::Array(vec![new_val]));
            })
            .is_err()
        {
            tracing::trace!(
                target: "suprnova::context",
                op = "push",
                "Context mutation discarded: no active scope on this task",
            );
        }
    }

    /// True if `key` is set in the visible bag.
    pub fn has(key: &str) -> bool {
        CONTEXT
            .try_with(|store| store.data.contains_key(key))
            .unwrap_or(false)
    }

    /// Remove `key` from both the visible and hidden bags.
    pub fn forget(key: &str) {
        if CONTEXT
            .try_with(|store| {
                store.data.remove(key);
                store.hidden.remove(key);
            })
            .is_err()
        {
            tracing::trace!(
                target: "suprnova::context",
                op = "forget",
                "Context mutation discarded: no active scope on this task",
            );
        }
    }

    /// Snapshot the visible bag. Returns an empty map outside a scope.
    pub fn all() -> HashMap<String, Value> {
        CONTEXT
            .try_with(|store| {
                store
                    .data
                    .iter()
                    .map(|kv| (kv.key().clone(), kv.value().clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Set `key` to `value` in the hidden bag (separate from the
    /// visible bag exposed by `all()`).
    pub fn hidden_add<K, V>(key: K, value: V)
    where
        K: Into<String>,
        V: Serialize,
    {
        if CONTEXT
            .try_with(|store| {
                let key = key.into();
                match serde_json::to_value(value) {
                    Ok(v) => {
                        store.hidden.insert(key, v);
                    }
                    Err(err) => {
                        tracing::trace!(
                            target: "suprnova::context",
                            op = "hidden_add",
                            key = ?key,
                            error = %err,
                            "Context mutation discarded: value failed to serialize",
                        );
                    }
                }
            })
            .is_err()
        {
            tracing::trace!(
                target: "suprnova::context",
                op = "hidden_add",
                "Context mutation discarded: no active scope on this task",
            );
        }
    }

    /// Read `key` from the hidden bag.
    ///
    /// Like [`Self::get`], a wrong-type read (`key` present but
    /// deserialize fails) emits a `tracing::trace!` so the bug is
    /// observable; plain absence stays silent.
    pub fn hidden_get<T: DeserializeOwned>(key: &str) -> Option<T> {
        CONTEXT
            .try_with(|store| {
                let raw = store.hidden.get(key)?;
                match serde_json::from_value::<T>(raw.value().clone()) {
                    Ok(v) => Some(v),
                    Err(err) => {
                        tracing::trace!(
                            target: "suprnova::context",
                            op = "hidden_get",
                            key = ?key,
                            expected = std::any::type_name::<T>(),
                            error = %err,
                            "Context read returned None: value present but did not deserialize",
                        );
                        None
                    }
                }
            })
            .ok()
            .flatten()
    }

    /// Read a query parameter from the current request.
    ///
    /// Resolution order:
    /// 1. The thread-local testing override (set via
    ///    `Self::test_set_query`) - non-empty only after a hook installs it.
    /// 2. The active [`CONTEXT`] scope's `query` bag - populated by
    ///    the request middleware from the URL's `?key=value` pairs.
    ///
    /// Returns `None` when the key is absent in both, including when
    /// called outside any context scope (the case for early-boot code,
    /// background workers without an installed scope, and tests
    /// without a query override).
    pub fn query_param(name: &str) -> Option<String> {
        // The per-thread testing override wins over the scoped query bag.
        // Without an installed override this branch misses and falls through.
        let from_override = QUERY_OVERRIDE.with(|cell| {
            cell.borrow()
                .as_ref()
                .and_then(|map| map.get(name).cloned())
        });
        if from_override.is_some() {
            return from_override;
        }

        CONTEXT
            .try_with(|store| store.query.get(name).map(|v| v.value().clone()))
            .ok()
            .flatten()
    }

    /// **Testing hook.** Install a query-parameter override on the current
    /// thread so [`Self::query_param`] reads it without requiring a
    /// wrapping [`CONTEXT::scope`][CONTEXT] call.
    ///
    /// Repeated calls overlay onto the same map. Use
    /// [`Self::test_clear_query`] to wipe between tests; otherwise an
    /// override from a previous `#[tokio::test]` body could leak into
    /// the next test scheduled onto the same OS thread (Cargo reuses
    /// threads across the per-binary thread pool).
    ///
    /// Compiled under `cfg(test)` or the `testing` Cargo feature. `testing` is
    /// enabled by default, including in release builds; consumers can remove
    /// this hook with `default-features = false` and without `testing`.
    #[cfg(any(test, feature = "testing"))]
    pub fn test_set_query(name: impl Into<String>, value: impl Into<String>) {
        QUERY_OVERRIDE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let map = slot.get_or_insert_with(HashMap::new);
            map.insert(name.into(), value.into());
        });
    }

    /// **Testing hook.** Wipe the thread-local query override. Pair with
    /// [`Self::test_set_query`] to keep tests on the same OS thread
    /// from leaking query params into each other.
    ///
    /// Compiled under `cfg(test)` or the `testing` Cargo feature. `testing` is
    /// enabled by default, including in release builds; consumers can remove
    /// this hook with `default-features = false` and without `testing`.
    #[cfg(any(test, feature = "testing"))]
    pub fn test_clear_query() {
        QUERY_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }

    /// **Testing hook.** Install a query-parameter override and return a
    /// guard that wipes the thread-local on drop.
    ///
    /// Preferred over the raw [`Self::test_set_query`] /
    /// [`Self::test_clear_query`] pair because it can't leak: even when
    /// the test body panics or returns early, the guard's `Drop` runs
    /// and clears the override before the OS thread is recycled by
    /// Cargo's per-binary thread pool.
    ///
    /// Repeated calls overlay onto the same map; the most recently
    /// returned guard wipes the override entirely when dropped (it does
    /// not restore the previous map). For most tests that's the right
    /// behavior - each `#[tokio::test]` sets up its own overrides from
    /// scratch.
    ///
    /// ```ignore
    /// #[tokio::test]
    /// async fn handler_reads_page_param() {
    ///     let _q = Context::test_query_guard("page", "3");
    ///     assert_eq!(Context::query_param("page"), Some("3".into()));
    ///     // `_q` drops at end of scope; the override is wiped.
    /// }
    /// ```
    ///
    /// Compiled under `cfg(test)` or the `testing` Cargo feature. `testing` is
    /// enabled by default, including in release builds; consumers can remove
    /// this hook with `default-features = false` and without `testing`.
    #[cfg(any(test, feature = "testing"))]
    #[must_use = "the guard wipes the test query override on drop; binding it to `_` clears immediately"]
    pub fn test_query_guard(name: impl Into<String>, value: impl Into<String>) -> TestQueryGuard {
        Self::test_set_query(name, value);
        TestQueryGuard { _private: () }
    }
}

/// **Testing hook.** RAII guard returned by
/// [`Context::test_query_guard`]; wipes the thread-local query
/// override on drop so a panicking or early-returning test body can't
/// leak overrides into the next test scheduled on the same OS thread.
///
/// Compiled under `cfg(test)` or the `testing` Cargo feature. `testing` is
/// enabled by default, including in release builds; consumers can remove this
/// type with `default-features = false` and without `testing`.
#[cfg(any(test, feature = "testing"))]
#[must_use = "the guard wipes the test query override on drop; binding it to `_` clears immediately"]
pub struct TestQueryGuard {
    // Private field so external crates can't construct one without
    // going through `Context::test_query_guard`, which is what installs
    // the override the guard is responsible for.
    _private: (),
}

#[cfg(any(test, feature = "testing"))]
impl Drop for TestQueryGuard {
    fn drop(&mut self) {
        Context::test_clear_query();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn add_and_get_round_trip_inside_scope() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("user_id", 42i64);
                assert_eq!(Context::get::<i64>("user_id"), Some(42));
                assert!(Context::has("user_id"));
                assert_eq!(Context::get::<String>("missing"), None);
            })
            .await;
    }

    #[tokio::test]
    async fn push_appends_to_a_stack() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::push("trail", json!("home"));
                Context::push("trail", json!("settings"));
                Context::push("trail", json!("billing"));
                let trail: Vec<String> = Context::get("trail").unwrap();
                assert_eq!(trail, vec!["home", "settings", "billing"]);
            })
            .await;
    }

    #[tokio::test]
    async fn forget_removes_key() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("k", "v");
                Context::forget("k");
                assert!(!Context::has("k"));
            })
            .await;
    }

    #[tokio::test]
    async fn hidden_storage_is_separate_from_visible() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("public_key", "yes");
                Context::hidden_add("secret_key", "shh");

                // all() returns visible only
                let all = Context::all();
                assert!(all.contains_key("public_key"));
                assert!(!all.contains_key("secret_key"));

                // hidden_get reads from hidden bag
                assert_eq!(
                    Context::hidden_get::<String>("secret_key"),
                    Some("shh".into())
                );
                assert_eq!(Context::hidden_get::<String>("public_key"), None);
            })
            .await;
    }

    #[tokio::test]
    #[serial_test::serial(context_hooks)]
    async fn dehydrate_outside_a_scope_has_nothing_to_carry() {
        Context::test_clear_hooks();
        assert_eq!(Context::dehydrate(), None);
    }

    #[tokio::test]
    #[serial_test::serial(context_hooks)]
    async fn dehydrate_copies_both_bags_and_leaves_the_query_behind() {
        Context::test_clear_hooks();
        let store = ContextStore::with_query([("page".to_owned(), "3".to_owned())].into());
        CONTEXT
            .scope(store, async {
                assert_eq!(Context::dehydrate(), None, "both bags are empty");

                Context::add("trace_id", "abc");
                Context::hidden_add("api_key", "s3cret");
                let snapshot = Context::dehydrate().expect("there is context to carry");

                assert_eq!(
                    snapshot.data,
                    [("trace_id".to_owned(), json!("abc"))].into()
                );
                assert_eq!(
                    snapshot.hidden,
                    [("api_key".to_owned(), json!("s3cret"))].into()
                );
                let wire = serde_json::to_string(&snapshot).unwrap();
                assert!(!wire.contains("page"), "the query bag travelled: {wire}");
            })
            .await;
    }

    #[tokio::test]
    #[serial_test::serial(context_hooks)]
    async fn restored_runs_on_a_copy() {
        Context::test_clear_hooks();
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("trace_id", "abc");
                Context::hidden_add("api_key", "s3cret");
                let snapshot = Context::dehydrate();

                Context::restored(snapshot, async {
                    assert_eq!(Context::get::<String>("trace_id").as_deref(), Some("abc"));
                    assert_eq!(
                        Context::hidden_get::<String>("api_key").as_deref(),
                        Some("s3cret")
                    );
                    Context::add("trace_id", "changed by the job");
                    Context::add("added_by_the_job", true);
                })
                .await;

                assert_eq!(
                    Context::get::<String>("trace_id").as_deref(),
                    Some("abc"),
                    "the job changed the pusher's context"
                );
                assert!(!Context::has("added_by_the_job"));
            })
            .await;
    }

    #[tokio::test]
    #[serial_test::serial(context_hooks)]
    async fn restored_without_a_snapshot_is_an_empty_scope() {
        Context::test_clear_hooks();
        Context::restored(None, async {
            assert!(Context::all().is_empty());
            Context::add("k", "v");
            assert_eq!(
                Context::get::<String>("k").as_deref(),
                Some("v"),
                "a job can use the context even when none was carried"
            );
        })
        .await;
    }

    #[tokio::test]
    #[serial_test::serial(context_hooks)]
    async fn the_hooks_run_at_both_ends_of_the_trip() {
        Context::test_clear_hooks();
        // The hooks are process-wide, and under plain `cargo test` other
        // tests take snapshots while these are registered. Both act only on
        // a snapshot that carries this test's marker.
        Context::dehydrating(|snapshot| {
            if snapshot.data.contains_key("hook_test") {
                snapshot.data.insert("locale".into(), json!("fr"));
                snapshot.hidden.remove("stays_home");
            }
        });
        Context::hydrated(|snapshot| {
            // Inside the restored scope: the facade writes the job's context.
            if let (true, Some(locale)) = (
                snapshot.data.contains_key("hook_test"),
                snapshot.data.get("locale"),
            ) {
                Context::add("locale_seen_by_the_worker", locale.clone());
            }
        });

        let snapshot = CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("hook_test", true);
                Context::hidden_add("stays_home", "yes");
                let snapshot = Context::dehydrate().expect("the hook added a value");
                assert!(
                    Context::hidden_get::<String>("stays_home").is_some(),
                    "a dehydrating hook changes the snapshot, never the live context"
                );
                assert!(!Context::has("locale"));
                snapshot
            })
            .await;
        assert_eq!(snapshot.data.get("locale"), Some(&json!("fr")));
        assert!(snapshot.hidden.is_empty());

        Context::restored(Some(snapshot), async {
            assert_eq!(
                Context::get::<String>("locale_seen_by_the_worker").as_deref(),
                Some("fr")
            );
        })
        .await;
        Context::test_clear_hooks();
    }

    #[test]
    fn the_debug_output_names_hidden_keys_and_never_their_values() {
        let snapshot = ContextSnapshot {
            data: [("trace_id".to_owned(), json!("abc"))].into(),
            hidden: [("api_key".to_owned(), json!("s3cret"))].into(),
        };
        let shown = format!("{snapshot:?}");
        assert!(shown.contains("trace_id") && shown.contains("abc"));
        assert!(shown.contains("api_key"));
        assert!(
            !shown.contains("s3cret"),
            "a hidden value was shown: {shown}"
        );
    }

    #[tokio::test]
    async fn the_debug_output_of_a_store_never_shows_a_hidden_value() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("trace_id", "abc");
                Context::hidden_add("api_key", "s3cret");
                let shown = format!("{:?}", Context::current().unwrap());
                assert!(shown.contains("trace_id") && shown.contains("abc"));
                assert!(shown.contains("api_key"));
                assert!(
                    !shown.contains("s3cret"),
                    "a hidden value was shown: {shown}"
                );
            })
            .await;
    }

    #[test]
    fn an_empty_bag_stays_off_the_wire() {
        let snapshot = ContextSnapshot {
            data: [("trace_id".to_owned(), json!("abc"))].into(),
            hidden: BTreeMap::new(),
        };
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            r#"{"data":{"trace_id":"abc"}}"#
        );
        let back: ContextSnapshot = serde_json::from_str("{}").unwrap();
        assert!(back.is_empty());
        assert_eq!(snapshot.without_hidden(), snapshot);
    }

    #[tokio::test]
    async fn outside_scope_operations_are_silent_noops() {
        // Calling Context::add outside a scope must not panic.
        Context::add("k", "v");
        assert_eq!(Context::get::<String>("k"), None);
        assert!(!Context::has("k"));
        assert!(Context::all().is_empty());
    }

    #[tokio::test]
    async fn query_param_reads_scoped_store() {
        // Wipe any override leaked from a sibling test on the same OS
        // thread - the per-thread override otherwise wins over the
        // scoped store and would mask a real read-from-scope bug.
        Context::test_clear_query();
        let mut q = HashMap::new();
        q.insert("page".to_string(), "3".to_string());
        q.insert("sort".to_string(), "name".to_string());
        let store = ContextStore::with_query(q);
        CONTEXT
            .scope(store, async {
                assert_eq!(Context::query_param("page"), Some("3".to_string()));
                assert_eq!(Context::query_param("sort"), Some("name".to_string()));
                assert_eq!(Context::query_param("missing"), None);
            })
            .await;
    }

    #[tokio::test]
    async fn query_param_outside_scope_is_none() {
        // Clear any override that may have leaked in from a previous
        // test on the same OS thread.
        Context::test_clear_query();
        assert_eq!(Context::query_param("page"), None);
    }

    #[tokio::test]
    async fn test_set_query_overrides_outside_scope() {
        Context::test_clear_query();
        Context::test_set_query("page", "7");
        assert_eq!(Context::query_param("page"), Some("7".to_string()));
        Context::test_clear_query();
        assert_eq!(Context::query_param("page"), None);
    }

    #[tokio::test]
    async fn test_set_query_overrides_scoped_store() {
        // The override should win even when a scope is installed.
        let mut q = HashMap::new();
        q.insert("page".to_string(), "1".to_string());
        let store = ContextStore::with_query(q);
        Context::test_clear_query();
        Context::test_set_query("page", "42");
        let result = CONTEXT
            .scope(store, async { Context::query_param("page") })
            .await;
        assert_eq!(result, Some("42".to_string()));
        Context::test_clear_query();
    }

    #[tokio::test]
    async fn current_returns_none_outside_scope() {
        assert!(Context::current().is_none());
    }

    #[tokio::test]
    async fn current_snapshots_active_store() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("k", "v");
                let snap = Context::current().expect("scope active");
                // The snapshot shares the same backing Arc, so writes
                // made *before* the snapshot are visible through it.
                assert_eq!(
                    snap.data.get("k").map(|v| v.value().clone()),
                    Some(json!("v")),
                );
            })
            .await;
    }

    #[tokio::test]
    async fn spawn_without_propagation_loses_context() {
        // Locks the gap the propagation helper closes: a bare
        // `tokio::spawn` inside a scope sees an empty `CONTEXT`.
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("request_id", "abc-123");

                let child = tokio::spawn(async {
                    // No scope inherited - reads see nothing.
                    Context::get::<String>("request_id")
                });

                let observed = child.await.expect("spawned task joined");
                assert_eq!(observed, None);
            })
            .await;
    }

    #[tokio::test]
    async fn spawn_with_current_then_scope_propagates_context() {
        // The recommended propagation pattern: snapshot via
        // `Context::current()` and re-enter via `Context::scope` in
        // the child. The shared `Arc<DashMap>` makes the parent's
        // writes visible to the child.
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("request_id", "abc-123");
                Context::hidden_add("user_id", 99i64);

                let store = Context::current().expect("scope active");
                let child = tokio::spawn(Context::scope(store, async {
                    (
                        Context::get::<String>("request_id"),
                        Context::hidden_get::<i64>("user_id"),
                    )
                }));

                let (rid, uid) = child.await.expect("spawned task joined");
                assert_eq!(rid, Some("abc-123".to_string()));
                assert_eq!(uid, Some(99));
            })
            .await;
    }

    #[tokio::test]
    async fn propagated_store_shares_query_bag() {
        // `query_param` reads should also flow through, since the
        // query bag is part of the same `ContextStore`.
        Context::test_clear_query();
        let mut q = HashMap::new();
        q.insert("page".to_string(), "5".to_string());
        let store = ContextStore::with_query(q);

        CONTEXT
            .scope(store, async {
                let snap = Context::current().expect("scope active");
                let child =
                    tokio::spawn(Context::scope(snap, async { Context::query_param("page") }));
                assert_eq!(child.await.unwrap(), Some("5".to_string()));
            })
            .await;
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn out_of_scope_mutations_emit_trace_event() {
        // Locks the "silent loss" gap: mutating ops still no-op as
        // documented, but they emit a `tracing::trace!` so the bug is
        // observable in instrumented runs.
        Context::add("k", "v");
        Context::push("stack", json!(1));
        Context::hidden_add("secret", "x");
        Context::forget("k");

        assert!(logs_contain("Context mutation discarded"));
        assert!(logs_contain("op=\"add\""));
        assert!(logs_contain("op=\"push\""));
        assert!(logs_contain("op=\"hidden_add\""));
        assert!(logs_contain("op=\"forget\""));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn in_scope_mutations_do_not_emit_trace_event() {
        // The trace event is gated on the no-scope branch; the happy
        // path stays silent.
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("k", "v");
                Context::push("stack", json!(1));
                Context::hidden_add("secret", "x");
                Context::forget("k");
            })
            .await;
        assert!(!logs_contain("Context mutation discarded"));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn get_wrong_type_returns_none_and_emits_trace_event() {
        // Locks the "wrong type" observability gap: get returns None
        // both for an absent key and for a present-but-wrong-type read;
        // the trace event lets callers distinguish the two in
        // instrumented runs.
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("user_id", 42i64);
                // i64 stored, requested as String: present but wrong type.
                assert_eq!(Context::get::<String>("user_id"), None);
            })
            .await;
        assert!(logs_contain("Context read returned None"));
        assert!(logs_contain("op=\"get\""));
        assert!(logs_contain("key=\"user_id\""));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn get_absent_key_stays_silent() {
        // Plain absence is the common case and would flood logs if it
        // emitted; only the present-but-wrong-type branch is observable.
        CONTEXT
            .scope(ContextStore::default(), async {
                assert_eq!(Context::get::<String>("never_set"), None);
            })
            .await;
        assert!(!logs_contain("Context read returned None"));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn hidden_get_wrong_type_emits_trace_event() {
        // Same observability contract for the hidden bag.
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::hidden_add("token_count", 7i64);
                assert_eq!(Context::hidden_get::<Vec<String>>("token_count"), None);
            })
            .await;
        assert!(logs_contain("Context read returned None"));
        assert!(logs_contain("op=\"hidden_get\""));
        assert!(logs_contain("key=\"token_count\""));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn hidden_get_absent_key_stays_silent() {
        CONTEXT
            .scope(ContextStore::default(), async {
                assert_eq!(Context::hidden_get::<String>("never_set"), None);
            })
            .await;
        assert!(!logs_contain("Context read returned None"));
    }

    /// A type whose `Serialize` impl deterministically fails - used to
    /// exercise the otherwise-hard-to-trigger `to_value` error path on
    /// `add` / `push` / `hidden_add`. Ordinary types almost never fail
    /// `serde_json::to_value`, but the observability contract holds for
    /// the ones that do (e.g. NaN floats serialized as strict JSON, or
    /// user-defined types with custom impls).
    struct AlwaysFailsSerialize;

    impl serde::Serialize for AlwaysFailsSerialize {
        fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("serialize intentionally fails"))
        }
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn add_serialize_failure_emits_trace_event() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::add("bad", AlwaysFailsSerialize);
                // Value was dropped; subsequent get must not find it.
                assert!(!Context::has("bad"));
            })
            .await;
        assert!(logs_contain("value failed to serialize"));
        assert!(logs_contain("op=\"add\""));
        assert!(logs_contain("key=\"bad\""));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn push_serialize_failure_emits_trace_event() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::push("bad_stack", AlwaysFailsSerialize);
                assert!(!Context::has("bad_stack"));
            })
            .await;
        assert!(logs_contain("value failed to serialize"));
        assert!(logs_contain("op=\"push\""));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn hidden_add_serialize_failure_emits_trace_event() {
        CONTEXT
            .scope(ContextStore::default(), async {
                Context::hidden_add("bad_secret", AlwaysFailsSerialize);
                assert_eq!(Context::hidden_get::<String>("bad_secret"), None);
            })
            .await;
        assert!(logs_contain("value failed to serialize"));
        assert!(logs_contain("op=\"hidden_add\""));
    }

    #[tokio::test]
    async fn test_query_guard_clears_on_drop() {
        // The RAII guard wipes the override even if the test body
        // doesn't call test_clear_query - the failure mode the LOW
        // finding flagged.
        Context::test_clear_query();
        {
            let _g = Context::test_query_guard("page", "11");
            assert_eq!(Context::query_param("page"), Some("11".to_string()));
        }
        // Guard dropped: override is gone.
        assert_eq!(Context::query_param("page"), None);
    }

    #[tokio::test]
    async fn test_query_guard_clears_on_scope_exit() {
        // The guard's Drop runs when its scope ends, on another thread
        // as well as this one. Verify the override didn't leak into the
        // current thread (we can't observe the spawned thread's
        // overrides anyway, so this also serves as a thread-isolation
        // sanity check).
        Context::test_clear_query();
        let handle = tokio::task::spawn_blocking(|| {
            let _g = Context::test_query_guard("page", "13");
            // Drop runs when the closure returns and the guard's scope
            // ends - no panic needed, Drop is unconditional.
            assert_eq!(Context::query_param("page"), Some("13".to_string()));
        });
        handle.await.expect("spawned blocking task completes");
        // The current thread never had the override installed.
        assert_eq!(Context::query_param("page"), None);
    }
}
