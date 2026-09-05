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
//! One test, and it is the one environment-mutating test this binary is
//! allowed (`live_boot.rs` documents that at most one test per binary may
//! call `std::env::set_var`). An earlier version mutated nothing and relied
//! on `APP_ENV` being unset so the framework detected `Local` and installed a
//! transient development key; a developer with `APP_ENV=production` exported
//! then got "APP_KEY is required when APP_ENV=production" from an assertion
//! whose message blamed the constructor's ordering (final review, F5). The
//! test now sets `APP_ENV=testing` and clears every key variable itself,
//! exactly as `live_boot.rs`'s
//! `runtime_is_bound_before_fallible_routes_and_reused_on_reentry` does, so
//! it does not depend on the invoking shell.

use std::sync::{Arc, Mutex};

use suprnova::live::LiveRuntime;
use suprnova::{App, FrameworkError, Router, Server};

#[tokio::test]
async fn the_live_runtime_is_bound_before_the_async_route_closure_runs() {
    // The only environment mutation in this binary; see the module doc.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::remove_var("APP_KEY");
        std::env::remove_var("APP_KEY_PREVIOUS");
        std::env::remove_var("APP_PREVIOUS_KEYS");
    }

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
