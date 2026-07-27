//! Central queue routing rules.
//!
//! Laravel 13 added `Queue::route(ProcessPodcast::class, connection: 'redis',
//! queue: 'podcasts')` so deployment-shaped decisions — which worker pool
//! drains which job — live in one place instead of being scattered across job
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
//! A registered route with a `None` field does not mask the job's own value —
//! only the fields you actually set take effect, so routing the connection
//! without disturbing the queue is expressible.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::error::FrameworkError;
use crate::queue::Job;

/// A routing rule: where a job's envelopes should be pushed.
///
/// Both fields are independent — `None` means "defer to the next source in
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
/// job dispatch down with it — falling back to the default queue keeps work
/// flowing while the poison is diagnosed.
pub(crate) fn route_for(job_name: &str) -> Option<QueueRoute> {
    let guard = crate::lock::read(registry(), LOCK_CONTEXT).ok()?;
    guard.get(job_name).cloned()
}

/// Remove every registered route. Test-support only: routes are global
/// process state, so a test that registers one must clear it to stay
/// hermetic. Compiled out of normal builds — routes are registered once at
/// boot and never torn down in a running process.
#[cfg(test)]
pub(crate) fn clear_routes() -> Result<(), FrameworkError> {
    crate::lock::write(registry(), LOCK_CONTEXT)?.clear();
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
}
