//! PAR-048: the redirect back of an empty response, fragment redirects,
//! the previous URL an Inertia visit records, and the `errors` shape
//! under `X-Inertia-Error-Bag`.
//!
//! Laravel's references are `Middleware::handle`, `onEmptyResponse`,
//! `onRedirectWithFragment`, `storeCurrentUrl` and
//! `resolveValidationErrors` in inertia-laravel 3.5.1.

use std::sync::Arc;

use serde_json::json;
use suprnova::http::text;
use suprnova::session::{SessionMiddleware, new_session_slot_for_test, session_scope_for_test};
use suprnova::{
    HttpResponse, Inertia, Inertia303Middleware, InertiaConfig, InertiaHeadersMiddleware,
    InertiaRequestExt, InertiaResponse, MiddlewareRegistry, Redirect, Request, Response, Router,
};

use crate::protocol_harness::{
    Client, MemoryStore, MockReq, SeededSessionScope, ensure_crypt, page_of, serve, session_config,
};

fn empty() -> Response {
    Ok(HttpResponse::new())
}

fn router() -> Router {
    Router::new()
        .post("/empty", |_req: Request| async { empty() })
        .put("/empty", |_req: Request| async { empty() })
        .get("/empty", |_req: Request| async { empty() })
        .get("/fragment", |_req: Request| async {
            let response: Response = Redirect::to("/page/Home#section").into();
            response
        })
        .put("/fragment", |_req: Request| async {
            let response: Response = Redirect::to("/page/Home#section").into();
            response
        })
        .get("/page/{name}", |req: Request| async move {
            let name = req.param("name").unwrap_or_default().to_owned();
            InertiaResponse::new(name)
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/plain", |_req: Request| async { text("plain") })
        .post("/back", |_req: Request| async {
            let response: Response = Inertia::back(302, Some("/fallback")).into();
            response
        })
        .post("/back-bare", |_req: Request| async {
            let response: Response = Inertia::back(303, None).into();
            response
        })
        .into()
}

/// The Inertia stack the redirect rules live in, inside a session the test
/// seeded.
fn stack(
    slot: &Arc<std::sync::Mutex<Option<suprnova::session::SessionData>>>,
) -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaHeadersMiddleware::new())
        .append(Inertia303Middleware::new())
}

fn previous_url(
    slot: &Arc<std::sync::Mutex<Option<suprnova::session::SessionData>>>,
) -> Option<String> {
    slot.lock().unwrap().as_ref().unwrap().previous_url()
}

// ---- an empty 200 becomes a redirect back ----

#[tokio::test]
async fn inp_an_empty_200_on_an_inertia_post_redirects_to_a_same_origin_referer_with_302() {
    // Laravel's `onEmptyResponse` is `Redirect::back()`: the `Referer`
    // first, status 302. A browser follows a 302 after POST with a GET, so
    // nothing needs the 303 the framework used to send for every method.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send(
            "POST",
            "/empty",
            &[
                ("X-Inertia", "true"),
                ("Referer", "http://localhost/form?step=2"),
            ],
        )
        .await;

    assert_eq!(reply.status, 302, "an empty 200 on a POST becomes a 302");
    assert_eq!(reply.header("location"), Some("/form?step=2"));
}

#[tokio::test]
async fn inp_an_empty_200_on_an_inertia_put_redirects_back_with_303() {
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send(
            "PUT",
            "/empty",
            &[("X-Inertia", "true"), ("Referer", "http://localhost/edit")],
        )
        .await;

    assert_eq!(reply.status, 303, "PUT, PATCH and DELETE get 303");
    assert_eq!(reply.header("location"), Some("/edit"));
}

#[tokio::test]
async fn inp_a_cross_origin_referer_falls_back_to_the_previous_url() {
    // The `Referer` lands in `Location`, so it passes the same-origin check
    // the validation redirect applies before it is used.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/dashboard");
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send(
            "POST",
            "/empty",
            &[
                ("X-Inertia", "true"),
                ("Referer", "https://evil.test/phish"),
            ],
        )
        .await;

    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/dashboard"));
}

// ---- Inertia::back follows the same order ----

#[tokio::test]
async fn inp_inertia_back_goes_to_the_referer_then_the_previous_url_then_the_fallback() {
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let mut client = Client::new(addr);

    let reply = client
        .send(
            "POST",
            "/back",
            &[("X-Inertia", "true"), ("Referer", "http://localhost/from")],
        )
        .await;
    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/from"), "the Referer first");

    let reply = client.send("POST", "/back", &[("X-Inertia", "true")]).await;
    assert_eq!(
        reply.header("location"),
        Some("/fallback"),
        "no Referer and no previous URL: the fallback"
    );

    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/before");
    let reply = client
        .send(
            "POST",
            "/back",
            &[("X-Inertia", "true"), ("Referer", "https://evil.test/x")],
        )
        .await;
    assert_eq!(
        reply.header("location"),
        Some("/before"),
        "a foreign Referer is skipped for the previous URL"
    );
}

#[tokio::test]
async fn inp_inertia_back_without_a_fallback_goes_to_the_root_with_its_status() {
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send("POST", "/back-bare", &[("X-Inertia", "true")])
        .await;
    assert_eq!(reply.status, 303);
    assert_eq!(reply.header("location"), Some("/"));
}

// ---- a fragment redirect becomes 409 + X-Inertia-Redirect ----

#[tokio::test]
async fn inp_a_redirect_with_a_fragment_on_an_inertia_visit_becomes_409_with_x_inertia_redirect() {
    // A `Location` fragment does not survive the XHR redirect the client
    // follows, so Laravel hands the target to the client as
    // `X-Inertia-Redirect`, which it visits with the fragment intact.
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send("GET", "/fragment", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 409);
    assert_eq!(
        reply.header("x-inertia-redirect"),
        Some("/page/Home#section")
    );
    assert_eq!(reply.header("location"), None);
    assert_eq!(reply.header("vary"), Some("X-Inertia"));
}

#[tokio::test]
async fn inp_a_fragment_redirect_after_put_is_409_too() {
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send("PUT", "/fragment", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 409);
    assert_eq!(
        reply.header("x-inertia-redirect"),
        Some("/page/Home#section")
    );
}

#[tokio::test]
async fn inp_a_fragment_redirect_on_a_prefetch_stays_a_redirect() {
    // A prefetch never navigates, so turning its redirect into a visit
    // instruction would be wrong. Guard: before the conversion existed this
    // passed trivially.
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send(
            "GET",
            "/fragment",
            &[("X-Inertia", "true"), ("Sec-Purpose", "prefetch")],
        )
        .await;

    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/page/Home#section"));
}

#[tokio::test]
async fn inp_a_fragment_redirect_on_a_plain_visit_stays_a_redirect() {
    let slot = new_session_slot_for_test();
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr).send("GET", "/fragment", &[]).await;

    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/page/Home#section"));
}

#[test]
fn inp_a_prefetch_is_read_from_x_moz_purpose_or_sec_purpose() {
    // `Request::prefetch()` in Laravel: any of the three headers equal to
    // `prefetch`, case-insensitively.
    assert!(
        MockReq::new("/")
            .header("Purpose", "prefetch")
            .is_prefetch()
    );
    assert!(
        MockReq::new("/")
            .header("Sec-Purpose", "prefetch")
            .is_prefetch()
    );
    assert!(MockReq::new("/").header("X-Moz", "Prefetch").is_prefetch());
    assert!(
        !MockReq::new("/")
            .header("Sec-Purpose", "prefetch;prerender")
            .is_prefetch()
    );
    assert!(!MockReq::new("/").is_prefetch());
}

// ---- the previous URL an Inertia visit records ----

#[tokio::test]
async fn inp_an_empty_200_on_an_inertia_get_records_its_url_after_the_redirect_chose_its_target() {
    // PAR-048 names no exemption for the empty response: the visit is
    // recorded. The redirect back is chosen first, so without a Referer it
    // goes to the page before this one rather than to this one.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send("GET", "/empty", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 302, "{reply:?}");
    assert_eq!(reply.header("location"), Some("/older"));
    assert_eq!(
        previous_url(&slot).as_deref(),
        Some("/empty"),
        "the empty GET is recorded like any other Inertia GET"
    );
}

#[tokio::test]
async fn inp_an_inertia_get_records_the_previous_url() {
    // Laravel's session middleware skips XHRs, so its Inertia middleware
    // records the visit itself; without it `back()` and a failed validation
    // land on whatever page was last loaded in full.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    let reply = Client::new(addr)
        .send("GET", "/page/Home?tab=2", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 200);
    assert_eq!(previous_url(&slot).as_deref(), Some("/page/Home?tab=2"));
}

#[tokio::test]
async fn inp_a_partial_reload_of_another_component_records_the_previous_url() {
    // A partial header naming a component the page did not render is a
    // navigation, not a reload of the page the visitor is on.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    Client::new(addr)
        .send(
            "GET",
            "/page/Home",
            &[
                ("X-Inertia", "true"),
                ("X-Inertia-Partial-Component", "Other"),
                ("X-Inertia-Partial-Data", "x"),
            ],
        )
        .await;

    assert_eq!(previous_url(&slot).as_deref(), Some("/page/Home"));
}

#[tokio::test]
async fn inp_a_partial_reload_of_the_same_component_keeps_the_previous_url() {
    // Deferred props, polling and infinite scroll reload the page the
    // visitor is on; none of them is a page the visitor came from.
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    Client::new(addr)
        .send(
            "GET",
            "/page/Home",
            &[
                ("X-Inertia", "true"),
                ("X-Inertia-Partial-Component", "Home"),
                ("X-Inertia-Partial-Data", "x"),
            ],
        )
        .await;

    assert_eq!(previous_url(&slot).as_deref(), Some("/older"));
}

#[tokio::test]
async fn inp_store_previous_url_off_keeps_the_previous_url() {
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let registry = MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaHeadersMiddleware::from_config(
            &InertiaConfig::new().store_previous_url(false),
        ));
    let addr = serve(router(), registry).await;
    Client::new(addr)
        .send("GET", "/page/Home", &[("X-Inertia", "true")])
        .await;

    assert_eq!(previous_url(&slot).as_deref(), Some("/older"));
}

#[tokio::test]
async fn inp_a_prefetch_or_precognitive_inertia_get_keeps_the_previous_url() {
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    let addr = serve(router(), stack(&slot)).await;
    let mut client = Client::new(addr);
    client
        .send(
            "GET",
            "/page/Home",
            &[("X-Inertia", "true"), ("Purpose", "prefetch")],
        )
        .await;
    client
        .send(
            "GET",
            "/page/Home",
            &[("X-Inertia", "true"), ("Precognition", "true")],
        )
        .await;

    assert_eq!(previous_url(&slot).as_deref(), Some("/older"));
}

#[tokio::test]
async fn inp_a_browser_prefetch_records_no_previous_url_in_the_session_middleware() {
    // The session middleware records a plain page load; a prefetch is no
    // page the visitor saw.
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let addr = serve(
        router(),
        MiddlewareRegistry::new().append(SessionMiddleware::with_store(
            session_config(),
            store.clone(),
        )),
    )
    .await;
    let mut client = Client::new(addr);
    client.send("GET", "/plain?first=1", &[]).await;
    client
        .send("GET", "/plain?second=1", &[("Sec-Purpose", "prefetch")])
        .await;

    assert_eq!(
        store.only_session().previous_url().as_deref(),
        Some("/plain?first=1")
    );
}

// ---- `errors` under X-Inertia-Error-Bag ----

async fn errors_with(seed: &[(&str, serde_json::Value)], bag: Option<&str>) -> serde_json::Value {
    let slot = new_session_slot_for_test();
    {
        let mut guard = slot.lock().unwrap();
        let session = guard.as_mut().unwrap();
        for (key, value) in seed {
            session.put(key, value.clone());
        }
    }
    let mut req = MockReq::new("/form").inertia();
    if let Some(bag) = bag {
        req = req.header("X-Inertia-Error-Bag", bag);
    }
    let response = session_scope_for_test(slot, async move {
        InertiaResponse::new("Form").resolve(&req).await.unwrap()
    })
    .await;
    page_of(response)["props"]["errors"].clone()
}

#[tokio::test]
async fn inp_the_error_bag_header_with_no_session_errors_gives_an_empty_object() {
    assert_eq!(errors_with(&[], Some("form")).await, json!({}));
}

#[tokio::test]
async fn inp_the_error_bag_header_wraps_the_default_bag() {
    let errors = errors_with(
        &[(
            "_flash.old.errors.default",
            json!({"email": ["The email is required."]}),
        )],
        Some("form"),
    )
    .await;
    assert_eq!(errors, json!({"form": {"email": "The email is required."}}));
}

#[tokio::test]
async fn inp_the_error_bag_header_without_a_default_bag_gives_the_named_bags_alone() {
    let errors = errors_with(
        &[
            ("_flash.old.errors.login", json!({"email": ["Wrong."]})),
            ("_flash.old.errors.signup", json!({"name": ["Taken."]})),
        ],
        Some("form"),
    )
    .await;
    assert_eq!(
        errors,
        json!({"login": {"email": "Wrong."}, "signup": {"name": "Taken."}})
    );
}
