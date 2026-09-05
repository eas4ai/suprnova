//! `Server::try_from_config_with_routes_async` must prepare framework
//! services and the immutable Live runtime *before* it runs the route
//! closure. That order is the whole reason the asynchronous hook exists:
//! `RenderCache::install`, the caller it was added for, probes the database
//! for the generation ledger's tables, which needs the booted container.
//!
//! `live_boot.rs` exercises the same constructor, but only after three
//! earlier boots in the same test have already bound the runtime, so an
//! observation made there resolves it whether the prologue ran first or
//! not. This file is its own process and boots nothing beforehand, so the
//! observation *is* the order.
//!
//! One test, and it mutates no environment. With `APP_ENV` unset the
//! framework detects `Local` and installs a transient development key, so
//! the constructor succeeds with no setup at all. That is deliberate:
//! `live_boot.rs` documents that at most one test in a binary may call
//! `std::env::set_var`, and needing none is the cheapest way to honour it.

use std::sync::{Arc, Mutex};

use suprnova::live::LiveRuntime;
use suprnova::{App, FrameworkError, Router, Server};

#[tokio::test]
async fn the_live_runtime_is_bound_before_the_async_route_closure_runs() {
    assert!(
        App::resolve::<LiveRuntime>().is_err(),
        "sanity: this process must start with nothing bound, or the observation \
         below would hold no matter what order the constructor used"
    );

    let observed = Arc::new(Mutex::new(None::<LiveRuntime>));
    let observed_in_routes = Arc::clone(&observed);

    let _server = Server::try_from_config_with_routes_async(move || async move {
        let runtime: LiveRuntime = App::resolve().map_err(|error| {
            FrameworkError::internal(format!(
                "Live runtime was not bound before asynchronous route construction: {error}"
            ))
        })?;
        *observed_in_routes.lock().expect("observation lock") = Some(runtime);
        Ok(Router::new())
    })
    .await
    .expect("the asynchronous constructor must prepare services and the Live runtime first");

    let during_routes = observed
        .lock()
        .expect("observation lock")
        .clone()
        .expect("the asynchronous route closure must have run");
    let after_routes: LiveRuntime =
        App::resolve().expect("the runtime stays container-bound after construction");
    assert!(
        suprnova::live::testing::same_runtime_instance(&during_routes, &after_routes),
        "the closure must observe the very runtime the constructor bound, not a \
         second one assembled behind it"
    );
}
