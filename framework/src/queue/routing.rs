//! Central queue routing rules.
//!
//! Laravel 13 added `Queue::route(ProcessPodcast::class, connection: 'redis',
//! queue: 'podcasts')` so deployment-shaped decisions - which worker pool
//! drains which job - live in one place instead of being scattered across job
//! definitions. This is the Suprnova equivalent.
//!
//! # Why a runtime registry rather than an attribute
//!
//! A job's queue is an operational decision, not a property of the code: the
//! same `SendInvoice` belongs on `default` in development and on a dedicated
//! `billing` pool in production. Suprnova's convention is that anything
//! needing runtime configuration is registered in `bootstrap::register()`
//! rather than declared at compile time (see the container docs), so routes
//! are registered there:
//!
//! ```rust,no_run
//! # use suprnova::queue::{Job, Queue};
//! # use suprnova::FrameworkError;
//! # #[derive(serde::Serialize, serde::Deserialize)]
//! # struct SendInvoice;
//! # #[suprnova::async_trait]
//! # impl Job for SendInvoice {
//! #     fn job_name() -> &'static str { "SendInvoice" }
//! #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
//! # }
//! pub async fn register() {
//!     Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
//! }
//! ```
//!
//! Resolution order, highest priority first:
//!
//! 1. a route registered here for the job's name
//! 2. the job's own [`Job::queue`] / [`Job::connection`]
//! 3. the driver / global default
//!
//! A registered route with a `None` field does not mask the job's own value -
//! only the fields you actually set take effect, so routing the connection
//! without disturbing the queue is expressible.
//!
//! Whatever that order produces is then run through the **forwards** map,
//! which is keyed by queue *name* rather than by job: `Queue::forward(
//! "default", "high")` moves every push that resolved to `default`, and moves
//! a worker started on `--queue=default` over to `high` as well. Forwards are
//! an operational redirect layered on top of routing, not a fourth priority
//! tier, and they resolve in a single lookup rather than chaining.
//!
//! The queue dimension is honored end to end - envelope, driver storage,
//! `queue:work --queue=...` filtering. The connection dimension currently
//! resolves the connection *name* reported on queue lifecycle events; driver
//! selection by connection is not implemented, and one process-global driver
//! receives every push.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::error::FrameworkError;
use crate::queue::Job;

/// A routing rule: where a job's envelopes should be pushed.
///
/// Both fields are independent - `None` means "defer to the next source in
/// the resolution order" rather than "use the default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueRoute {
    /// Connection name, or `None` to defer to the job / global default.
    pub connection: Option<String>,
    /// Queue name, or `None` to defer to the job / driver default.
    pub queue: Option<String>,
}

fn registry() -> &'static RwLock<HashMap<&'static str, QueueRoute>> {
    static REGISTRY: OnceLock<RwLock<HashMap<&'static str, QueueRoute>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

const LOCK_CONTEXT: &str = "queue route registry";

/// Register (or replace) the routing rule for `J`.
///
/// Registering the same job twice replaces the earlier rule, so a test or a
/// per-environment bootstrap can override a default without unregistering.
pub(crate) fn try_set_route<J: Job>(
    connection: Option<&str>,
    queue: Option<&str>,
) -> Result<(), FrameworkError> {
    let route = QueueRoute {
        connection: connection.map(str::to_owned),
        queue: queue.map(str::to_owned),
    };
    crate::lock::write(registry(), LOCK_CONTEXT)?.insert(J::job_name(), route);
    Ok(())
}

/// The rule registered for `job_name`, if any.
///
/// Reads degrade to "no route" on a poisoned registry rather than failing.
/// This runs on the push path, and a poisoned routing table must not take
/// job dispatch down with it - falling back to the default queue keeps work
/// flowing while the poison is diagnosed.
pub(crate) fn route_for(job_name: &str) -> Option<QueueRoute> {
    let guard = crate::lock::read(registry(), LOCK_CONTEXT).ok()?;
    guard.get(job_name).cloned()
}

/// Remove every registered route. Test-support only: routes are global
/// process state, so a test that registers one must clear it to stay
/// hermetic. Compiled out of normal builds - routes are registered once at
/// boot and never torn down in a running process.
#[cfg(test)]
pub(crate) fn clear_routes() -> Result<(), FrameworkError> {
    crate::lock::write(registry(), LOCK_CONTEXT)?.clear();
    Ok(())
}

fn forwards() -> &'static RwLock<HashMap<String, QueueRoute>> {
    static FORWARDS: OnceLock<RwLock<HashMap<String, QueueRoute>>> = OnceLock::new();
    FORWARDS.get_or_init(|| RwLock::new(HashMap::new()))
}

const FORWARD_LOCK_CONTEXT: &str = "queue forward registry";

/// Whether registering `from -> to` would close a loop in `map`.
///
/// Walks the destinations already registered, starting at `to`; a walk that
/// arrives back at `from` is a loop. The entry being replaced is never
/// followed, because reaching `from` at all is precisely the answer `true`.
/// `from == to` is the identity rather than a loop - see [`forwarded_queue`] -
/// so it is reported as no cycle. The walk is bounded by the map's size: the
/// existing graph is acyclic by induction, so a path visits at most that many
/// distinct names, and the bound holds even if that invariant were ever broken.
fn forms_cycle(map: &HashMap<String, QueueRoute>, from: &str, to: &str) -> bool {
    if from == to {
        return false;
    }
    let mut hop = to;
    for _ in 0..map.len() {
        let Some(next) = map.get(hop).and_then(|f| f.queue.as_deref()) else {
            return false;
        };
        if next == from {
            return true;
        }
        if next == hop {
            // A self-forward is the identity, so the chain ends here.
            return false;
        }
        hop = next;
    }
    false
}

/// Register (or replace) the forward for the queue named `from`.
///
/// `connection` gates the forward: `None` applies it on every connection,
/// `Some(name)` only when the push (or the worker) is on that connection.
/// Registering the same source twice replaces the earlier forward.
///
/// A forward that would close a loop (`a -> b` with `b -> a` already
/// registered) is refused. Forwards resolve in a single lookup and never
/// chain, so a loop cannot mean what the operator who wrote it intended;
/// refusing it at registration says so, where accepting it would quietly
/// resolve to something else. Forwarding a queue onto its own name is the
/// identity, not a loop, and stays legal.
pub(crate) fn try_set_forward(
    from: &str,
    to: &str,
    connection: Option<&str>,
) -> Result<(), FrameworkError> {
    let forward = QueueRoute {
        connection: connection.map(str::to_owned),
        queue: Some(to.to_owned()),
    };
    let mut guard = crate::lock::write(forwards(), FORWARD_LOCK_CONTEXT)?;
    if forms_cycle(&guard, from, to) {
        return Err(FrameworkError::internal(format!(
            "queue forward `{from}` -> `{to}` would create a cycle; forwards \
             resolve in a single lookup and never chain, so a cycle cannot \
             redirect anywhere"
        )));
    }
    guard.insert(from.to_owned(), forward);
    Ok(())
}

/// The forward registered for the queue named `from`, if any.
///
/// Reads degrade to "no forward" on a poisoned registry, matching
/// [`route_for`] and for the same reason: this runs on the push path, and a
/// poisoned routing table must not take job dispatch down with it.
pub(crate) fn forward_for(from: &str) -> Option<QueueRoute> {
    let guard = crate::lock::read(forwards(), FORWARD_LOCK_CONTEXT).ok()?;
    guard.get(from).cloned()
}

/// Whether any forward is registered at all.
///
/// The push path calls this before doing anything else, so a deployment that
/// never forwards pays one uncontended read lock instead of a connection
/// resolution plus a map lookup on every envelope. Laravel takes the same
/// shortcut in `QueueRoutes::getConnection` (`if (empty($this->forwards))`).
pub(crate) fn has_forwards() -> bool {
    crate::lock::read(forwards(), FORWARD_LOCK_CONTEXT)
        .map(|g| !g.is_empty())
        .unwrap_or(false)
}

/// Apply the registered forwards to one queue name.
///
/// `None` means "the driver's default queue", so it is looked up as
/// [`DEFAULT_QUEUE`](crate::queue::envelope::DEFAULT_QUEUE) - Laravel resolves
/// `$queue ?: $this->default` before forwarding, which is what makes
/// `forward("default", "high")` catch jobs that named no queue. When nothing
/// matches, the input is returned unchanged, `None` included, so an unrouted
/// envelope keeps the absent `queue` key that makes it byte-identical to what
/// pre-routing versions wrote.
///
/// A forward onto the queue's own name is treated as no redirect at all, for
/// the same wire-format reason: `forward("default", "default")` expresses the
/// identity, and turning a `None` queue into `Some("default")` would change
/// what an unrouted envelope puts on the wire while changing nothing about
/// where it is drained.
///
/// A single lookup, never a chain: with `a -> b` and `b -> c` registered, a
/// push for `a` lands on `b`. Laravel's `forwardedQueue` behaves the same way.
/// A forward that would close a loop is refused by [`try_set_forward`], so this
/// resolver never walks a graph and never has to defend against one.
pub(crate) fn forwarded_queue(queue: Option<&str>, connection: &str) -> Option<String> {
    if !has_forwards() {
        return queue.map(str::to_owned);
    }
    let lookup = queue.unwrap_or(crate::queue::envelope::DEFAULT_QUEUE);
    let Some(forward) = forward_for(lookup) else {
        return queue.map(str::to_owned);
    };
    let applies = match forward.connection.as_deref() {
        None => true,
        Some(gate) => gate == connection,
    };
    match forward.queue {
        Some(destination) if applies && destination != lookup => Some(destination),
        _ => queue.map(str::to_owned),
    }
}

/// Rewrite a worker's claim list through the registered forwards.
///
/// This is the half that keeps a forward from stranding work. Laravel gets it
/// for free because every driver's `getQueue()` runs on the pop path as well as
/// the push path; Suprnova's worker hands `--queue` names to
/// [`QueueDriver::pop_from`](crate::queue::QueueDriver::pop_from) verbatim, so
/// the rewrite is explicit here.
///
/// An empty list means "drain every queue the driver holds" - no forward can
/// strand work against that, and there is nothing to rewrite, so it is returned
/// untouched. Two sources forwarding to one destination collapse to a single
/// entry: order is preserved, because the memory and database drivers scan the
/// claim list in order and a duplicate would silently change nothing while
/// looking like it did.
pub(crate) fn forward_active_queues(connection: &str, queues: Vec<String>) -> Vec<String> {
    if queues.is_empty() || !has_forwards() {
        return queues;
    }
    let mut out: Vec<String> = Vec::with_capacity(queues.len());
    for queue in queues {
        let resolved = forwarded_queue(Some(&queue), connection);
        let dest = resolved.unwrap_or(queue);
        if !out.contains(&dest) {
            out.push(dest);
        }
    }
    out
}

/// Remove every registered forward. Test-support only, for the same reason
/// [`clear_routes`] is: forwards are global process state.
#[cfg(test)]
pub(crate) fn clear_forwards() -> Result<(), FrameworkError> {
    crate::lock::write(forwards(), FORWARD_LOCK_CONTEXT)?.clear();
    Ok(())
}

/// Resolve the queue an envelope for `J` should carry.
///
/// Returns `None` for "the driver's default queue", which is what the
/// envelope stores when nothing routes the job.
pub(crate) fn resolve_queue<J: Job>() -> Option<String> {
    if let Some(route) = route_for(J::job_name())
        && route.queue.is_some()
    {
        return route.queue;
    }
    J::queue().map(str::to_owned)
}

/// Resolve the connection name for `J`, falling back to `global` when
/// neither a route nor the job expresses a preference.
pub(crate) fn resolve_connection<J: Job>(global: String) -> String {
    if let Some(route) = route_for(J::job_name())
        && let Some(connection) = route.connection
    {
        return connection;
    }
    J::connection().map(str::to_owned).unwrap_or(global)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FrameworkError;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Unit;

    #[crate::async_trait]
    impl Job for Unit {
        fn job_name() -> &'static str {
            "routing::unit::Unit"
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            Ok(())
        }
        fn queue() -> Option<&'static str> {
            Some("declared")
        }
        fn connection() -> Option<&'static str> {
            Some("declared-conn")
        }
    }

    /// These share the process-global registry, so they run as one test to
    /// stay hermetic under parallel execution rather than racing each other.
    #[test]
    fn route_precedence_and_clearing() {
        clear_routes().expect("clear");

        // Nothing registered: the job's own declarations win.
        assert_eq!(resolve_queue::<Unit>().as_deref(), Some("declared"));
        assert_eq!(resolve_connection::<Unit>("global".into()), "declared-conn");

        // A route outranks the job.
        try_set_route::<Unit>(Some("routed-conn"), Some("routed")).expect("set");
        assert_eq!(resolve_queue::<Unit>().as_deref(), Some("routed"));
        assert_eq!(resolve_connection::<Unit>("global".into()), "routed-conn");

        // A partially-specified route only overrides what it sets.
        try_set_route::<Unit>(None, Some("routed-only")).expect("set");
        assert_eq!(resolve_queue::<Unit>().as_deref(), Some("routed-only"));
        assert_eq!(
            resolve_connection::<Unit>("global".into()),
            "declared-conn",
            "a None connection defers to the job, it does not reset to global"
        );

        // Clearing restores the job's own view.
        clear_routes().expect("clear");
        assert_eq!(resolve_queue::<Unit>().as_deref(), Some("declared"));
        assert_eq!(route_for(Unit::job_name()), None);
    }

    /// A job with no opinion and no route falls all the way through to the
    /// caller-supplied global, which is what keeps existing deployments on
    /// their current connection after upgrading.
    #[test]
    fn unopinionated_job_falls_through_to_the_global() {
        #[derive(Serialize, Deserialize)]
        struct Bare;

        #[crate::async_trait]
        impl Job for Bare {
            fn job_name() -> &'static str {
                "routing::unit::Bare"
            }
            async fn handle(self) -> Result<(), FrameworkError> {
                Ok(())
            }
        }

        assert_eq!(resolve_queue::<Bare>(), None);
        assert_eq!(resolve_connection::<Bare>("global".into()), "global");
    }

    /// These share the process-global forwards registry, so they run as one
    /// test to stay hermetic under parallel execution, the same way
    /// `route_precedence_and_clearing` above does for routes.
    #[test]
    fn forward_resolution_and_clearing() {
        clear_forwards().expect("clear");

        // No forward registered: the name passes through, `None` included.
        assert_eq!(forwarded_queue(Some("a"), "conn").as_deref(), Some("a"));
        assert_eq!(forwarded_queue(None, "conn"), None);
        assert!(!has_forwards());

        try_set_forward("a", "b", None).expect("set");
        assert!(has_forwards());
        assert_eq!(forwarded_queue(Some("a"), "conn").as_deref(), Some("b"));
        assert_eq!(
            forwarded_queue(Some("b"), "conn").as_deref(),
            Some("b"),
            "a forward is a single lookup, never a chain"
        );

        // An unnamed queue means `default`, which a forward on `default` catches.
        try_set_forward("default", "d", None).expect("set");
        assert_eq!(forwarded_queue(None, "conn").as_deref(), Some("d"));

        // A connection-scoped forward fires only on its own connection.
        try_set_forward("c", "c-dest", Some("redis")).expect("set");
        assert_eq!(
            forwarded_queue(Some("c"), "redis").as_deref(),
            Some("c-dest")
        );
        assert_eq!(
            forwarded_queue(Some("c"), "database").as_deref(),
            Some("c"),
            "a forward gated on another connection must be inert, not partially applied"
        );

        // The worker's claim list follows, deduped, order preserved, and an
        // unfiltered worker is left alone.
        try_set_forward("e", "b", None).expect("set");
        assert_eq!(
            forward_active_queues("conn", vec!["a".into(), "e".into(), "z".into()]),
            vec!["b".to_string(), "z".to_string()],
            "two sources forwarding to one destination collapse to one claim"
        );
        assert_eq!(
            forward_active_queues("conn", Vec::new()),
            Vec::<String>::new(),
            "an unfiltered worker drains everything; there is nothing to rewrite"
        );

        // A loop is what an operator writes when they expect forwards to
        // chain. They never do, so the configuration cannot mean what it looks
        // like and is refused at registration rather than resolving to
        // something else.
        let err = try_set_forward("b", "a", None)
            .expect_err("a forward that closes a loop must be refused");
        assert!(
            err.to_string().contains("cycle"),
            "the error must name the cycle rather than just failing: {err}"
        );
        assert_eq!(
            forwarded_queue(Some("b"), "conn").as_deref(),
            Some("b"),
            "a refused forward must leave nothing half-registered"
        );

        // One hop onto the queue's own name is the identity, not a loop: it
        // stays legal, because it is the only way to neutralise a forward that
        // is already registered.
        try_set_forward("default", "default", None).expect("set");
        assert_eq!(
            forwarded_queue(None, "conn"),
            None,
            "a queue forwarded onto itself is not redirected, so an envelope \
             that named no queue still names none"
        );

        clear_forwards().expect("clear");
        assert!(!has_forwards());
        assert_eq!(forwarded_queue(Some("a"), "conn").as_deref(), Some("a"));
    }
}
