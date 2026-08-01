//! The application's route table.
//!
//! The routes themselves live in [`all`]; this module exists only to wrap
//! them.
//!
//! `routes!` generates a `pub fn register()` of its own, and its entries
//! are parsed as expressions — so `#[cfg(feature = "bench")]` cannot be
//! attached to one, and a second `register()` cannot sit beside it.
//! Putting the macro in a private child module leaves room here for a
//! wrapper that adds the benchmark group when, and only when, the feature
//! is on.

use suprnova::Router;

mod all;

/// Build the router.
pub fn register() -> Router {
    with_bench_routes(all::register())
}

/// Mount `/bench/*` (see [`crate::controllers::bench`]).
#[cfg(feature = "bench")]
fn with_bench_routes(router: Router) -> Router {
    crate::controllers::bench::register(router)
}

/// Identity without the feature. The benchmark routes are not merely
/// unreachable in a default build — `controllers::bench` is not compiled,
/// so there is nothing to reach.
#[cfg(not(feature = "bench"))]
fn with_bench_routes(router: Router) -> Router {
    router
}
