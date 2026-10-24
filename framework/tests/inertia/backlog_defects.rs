//! The backlog defects round's Inertia items: the previous URL of a visit
//! the fallback route answers (PAR-048), and the cursor and the page name
//! the Eloquent builder's paginators carry into the scroll metadata
//! (PAR-052).
//!
//! Laravel's references are `Middleware::shouldStoreCurrentUrl` in
//! inertia-laravel 3.5.1, which reads a fallback route as a matched
//! `Route` (`Router::fallback` registers one), and
//! `ScrollMetadata::fromPaginator`, which reads the current cursor and the
//! page name off the paginator.

use std::sync::Arc;

use serde_json::{Value, json};
use suprnova::context::Context;
use suprnova::http::text;
use suprnova::session::{SessionData, new_session_slot_for_test};
use suprnova::testing::TestDatabase;
use suprnova::{
    HttpResponse, Inertia303Middleware, InertiaConfig, InertiaHeadersMiddleware, InertiaResponse,
    MiddlewareRegistry, Model, ProvidesScrollMetadata, Request, Router, attrs, model,
};

use crate::protocol_harness::{Client, MockReq, SeededSessionScope, ensure_crypt, page_of, serve};

type Slot = Arc<std::sync::Mutex<Option<SessionData>>>;

// ---- PAR-048: a visit the fallback route answers records the previous URL ----

/// A router with one route and a fallback that renders an Inertia page.
/// The fallback's page carries whether its request had a route pattern,
/// so a test can see that the fallback request is no routed match for
/// the readers of the pattern (RenderCache, the debug page, DevTools).
fn fallback_router() -> Router {
    let router: Router = Router::new()
        .get("/known", |_req: Request| async { text("known") })
        .into();
    suprnova::fallback!(|req: Request| async move {
        let pattern = req.route_pattern().map(str::to_string);
        let name = req.route_name();
        InertiaResponse::new("Missing")
            .with("pattern", pattern)
            .with("name", name)
            .resolve(&req)
            .await
            .map_err(HttpResponse::from)
    })
    .register(router)
}

fn stack(slot: &Slot, config: &InertiaConfig) -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaHeadersMiddleware::from_config(config))
        .append(Inertia303Middleware::new())
}

fn seeded_slot() -> Slot {
    let slot = new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .set_previous_url("/older");
    slot
}

fn previous_url(slot: &Slot) -> Option<String> {
    slot.lock().unwrap().as_ref().unwrap().previous_url()
}

#[tokio::test]
async fn inp_an_inertia_get_the_fallback_answers_records_the_previous_url() {
    let slot = seeded_slot();
    let addr = serve(fallback_router(), stack(&slot, &InertiaConfig::new())).await;
    let reply = Client::new(addr)
        .send("GET", "/no/such/page?tab=2", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(reply.page()["component"], "Missing");
    assert_eq!(
        previous_url(&slot).as_deref(),
        Some("/no/such/page?tab=2"),
        "Laravel's shouldStoreCurrentUrl reads the fallback route as a matched route"
    );
}

#[tokio::test]
async fn inp_the_fallback_request_carries_no_route_pattern_and_no_route_name() {
    // The marker that lets the visit record the previous URL is not a
    // route pattern: RenderCache keys its policy on the pattern, and the
    // debug page and DevTools show it as the matched route.
    let slot = seeded_slot();
    let addr = serve(fallback_router(), stack(&slot, &InertiaConfig::new())).await;
    let reply = Client::new(addr)
        .send("GET", "/elsewhere", &[("X-Inertia", "true")])
        .await;

    let page = reply.page();
    assert_eq!(page["props"]["pattern"], Value::Null, "{page}");
    assert_eq!(page["props"]["name"], Value::Null, "{page}");
}

#[tokio::test]
async fn inp_a_fallback_visit_that_is_a_prefetch_or_a_partial_reload_keeps_the_previous_url() {
    let slot = seeded_slot();
    let addr = serve(fallback_router(), stack(&slot, &InertiaConfig::new())).await;
    let mut client = Client::new(addr);

    let prefetch = client
        .send(
            "GET",
            "/prefetched",
            &[("X-Inertia", "true"), ("Purpose", "prefetch")],
        )
        .await;
    assert_eq!(prefetch.status, 200, "{prefetch:?}");
    assert_eq!(previous_url(&slot).as_deref(), Some("/older"));

    let partial = client
        .send(
            "GET",
            "/reloaded",
            &[
                ("X-Inertia", "true"),
                ("X-Inertia-Partial-Component", "Missing"),
                ("X-Inertia-Partial-Data", "pattern"),
            ],
        )
        .await;
    assert_eq!(partial.status, 200, "{partial:?}");
    assert_eq!(
        previous_url(&slot).as_deref(),
        Some("/older"),
        "a partial reload of the page the fallback rendered is no new location"
    );
}

#[tokio::test]
async fn inp_a_fallback_visit_records_nothing_with_store_previous_url_off() {
    let slot = seeded_slot();
    let config = InertiaConfig::new().store_previous_url(false);
    let addr = serve(fallback_router(), stack(&slot, &config)).await;
    let reply = Client::new(addr)
        .send("GET", "/no/such/page", &[("X-Inertia", "true")])
        .await;

    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(previous_url(&slot).as_deref(), Some("/older"));
}

#[tokio::test]
async fn inp_an_inertia_get_records_its_previous_url_with_the_query_normalised() {
    // Laravel's storeCurrentUrl stores `$request->fullUrl()`: the query
    // sorted by key and re-encoded per RFC 3986, the spelling the page
    // `url` and the version 409's location carry (PAR-056).
    let slot = seeded_slot();
    let addr = serve(fallback_router(), stack(&slot, &InertiaConfig::new())).await;
    let mut client = Client::new(addr);

    let routed = client
        .send("GET", "/known?b=2&a=1+x", &[("X-Inertia", "true")])
        .await;
    assert_eq!(routed.status, 200, "{routed:?}");
    assert_eq!(previous_url(&slot).as_deref(), Some("/known?a=1%20x&b=2"));

    let fallback = client
        .send("GET", "/missing?z=1&y=2", &[("X-Inertia", "true")])
        .await;
    assert_eq!(fallback.page()["url"], "/missing?y=2&z=1");
    assert_eq!(previous_url(&slot).as_deref(), Some("/missing?y=2&z=1"));

    // A query that normalises to nothing leaves the path alone.
    client
        .send("GET", "/known?&", &[("X-Inertia", "true")])
        .await;
    assert_eq!(previous_url(&slot).as_deref(), Some("/known"));
}

// ---- PAR-052: the cursor and the page name of the builder's paginators ----

#[model(table = "bd_posts", fillable = ["title"])]
pub struct BdPost {
    pub id: i64,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A database holding `n` posts, with the cursor key ring installed and no
/// query parameter left over from another test on this thread.
async fn posts(n: usize) -> TestDatabase {
    Context::test_clear_query();
    ensure_crypt();
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    db.execute_unprepared(
        "CREATE TABLE bd_posts (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            title TEXT NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL\
         )",
    )
    .await
    .expect("create bd_posts");
    for i in 0..n {
        BdPost::create(attrs! { title: format!("p{i}") })
            .await
            .expect("seed bd_posts");
    }
    db
}

/// The scroll metadata the Inertia page object carries for `paginator`
/// under the prop `posts`.
async fn scroll_props<P>(paginator: P) -> Value
where
    P: suprnova::IntoInertiaScroll<BdPost>,
{
    let response = InertiaResponse::new("Feed")
        .paginate("posts", paginator)
        .resolve(&MockReq::new("/").inertia())
        .await
        .expect("the page resolves");
    page_of(response)["scrollProps"]["posts"].clone()
}

#[tokio::test]
async fn inp_a_cursor_page_records_the_cursor_it_was_fetched_with() {
    let _db = posts(5).await;
    let first = BdPost::query().cursor_paginate(2).await.unwrap();
    assert_eq!(first.current_cursor, None, "the first page has no cursor");
    let cursor = first.next_cursor.clone().expect("a second page");

    let second = {
        let _query = Context::test_query_guard("cursor", cursor.clone());
        BdPost::query().cursor_paginate(2).await.unwrap()
    };

    assert_eq!(second.current_cursor.as_deref(), Some(cursor.as_str()));
    // The request's parameter is gone; the metadata reads the paginator.
    assert_eq!(second.scroll_metadata().current_page, Some(json!(cursor)));
    let scroll = scroll_props(second).await;
    assert_eq!(scroll["pageName"], "cursor");
    assert_eq!(scroll["currentPage"], json!(cursor));
}

#[tokio::test]
async fn inp_a_tampered_cursor_fails_with_400_and_yields_no_page() {
    let _db = posts(3).await;
    let _query = Context::test_query_guard("cursor", "not-a-cursor");
    let error = BdPost::query()
        .cursor_paginate(2)
        .await
        .expect_err("a cursor that does not decode");
    assert_eq!(error.status_code(), 400);
}

#[tokio::test]
async fn inp_simple_paginate_using_reads_its_page_name_and_carries_it() {
    let _db = posts(5).await;
    let page = {
        let _query = Context::test_query_guard("comments_page", "2");
        BdPost::query()
            .order_by_asc("id")
            .simple_paginate_using("comments_page", 2)
            .await
            .unwrap()
    };

    assert_eq!(page.current_page, 2);
    assert_eq!(page.page_name(), "comments_page");
    let titles: Vec<&str> = page.data.iter().map(|post| post.title.as_str()).collect();
    assert_eq!(titles, ["p2", "p3"]);
    assert_eq!(page.url_for_page(3), "?comments_page=3");
    assert_eq!(page.next_page_url().as_deref(), Some("?comments_page=3"));
    let scroll = scroll_props(page).await;
    assert_eq!(
        scroll,
        json!({
            "pageName": "comments_page",
            "previousPage": 1,
            "nextPage": 3,
            "currentPage": 2,
            "reset": false,
        })
    );
}

#[tokio::test]
async fn inp_simple_paginate_using_ignores_the_page_parameter_of_another_listing() {
    // `?page=2` belongs to another paginator on the page; this one reads
    // only its own key and is on its first page.
    let _db = posts(5).await;
    let page = {
        let _query = Context::test_query_guard("page", "2");
        BdPost::query()
            .order_by_asc("id")
            .simple_paginate_using("comments_page", 2)
            .await
            .unwrap()
    };
    assert_eq!(page.current_page, 1);
    assert_eq!(page.data[0].title, "p0");

    // The shipped method still reads `page`, and names it.
    let shipped = {
        let _query = Context::test_query_guard("page", "2");
        BdPost::query()
            .order_by_asc("id")
            .simple_paginate(2)
            .await
            .unwrap()
    };
    assert_eq!(shipped.current_page, 2);
    assert_eq!(shipped.page_name(), "page");
    assert_eq!(scroll_props(shipped).await["pageName"], "page");
}

#[tokio::test]
async fn inp_simple_paginate_using_refuses_a_zero_page_size() {
    let _db = posts(1).await;
    let error = BdPost::query()
        .simple_paginate_using("comments_page", 0)
        .await
        .expect_err("a page of no rows");
    assert_eq!(error.status_code(), 400);
}

#[tokio::test]
async fn inp_the_fallback_route_is_marked_on_its_request_alone() {
    // `Request::is_fallback_route` reads the mark the server sets on the
    // fallback request; a matched route and a test request carry none.
    let router: Router = Router::new()
        .get("/known", |req: Request| async move {
            text(format!("fallback={}", req.is_fallback_route()))
        })
        .into();
    let router = suprnova::fallback!(|req: Request| async move {
        text(format!("fallback={}", req.is_fallback_route()))
    })
    .register(router);
    let slot = seeded_slot();
    let addr = serve(router, stack(&slot, &InertiaConfig::new())).await;
    let mut client = Client::new(addr);

    assert_eq!(
        client.send("GET", "/known", &[]).await.body,
        "fallback=false"
    );
    assert_eq!(
        client.send("GET", "/unknown", &[]).await.body,
        "fallback=true"
    );
    assert!(!Request::for_test("GET", "/unknown").is_fallback_route());
}
