//! PAR-050: Inertia flash data lives in the session under
//! `inertia.flash_data`, survives any number of redirects and is pulled by
//! the page that shows it; the `clear_history` and `preserve_fragment` flags
//! last until a page emits them.
//!
//! Laravel's references are `ResponseFactory::flash`, `getFlashed`,
//! `pullFlashed`, `clearHistory`, `preserveFragment`, `Response::__construct`
//! and `Middleware::reflash` in inertia-laravel 3.5.1.

use std::sync::Arc;

use serde_json::json;
use suprnova::http::text;
use suprnova::session::SessionMiddleware;
use suprnova::{
    App, FlashKey, HttpResponse, Inertia, Inertia303Middleware, InertiaHeadersMiddleware,
    InertiaResponse, MiddlewareRegistry, Redirect, Request, Response, Router,
};

/// An application's own key type, as a Laravel app flashes with an enum.
enum Toast {
    Success,
}

impl FlashKey for Toast {
    fn flash_key(&self) -> String {
        match self {
            Toast::Success => "success".to_string(),
        }
    }
}

use crate::protocol_harness::{Client, MemoryStore, ensure_crypt, serve, session_config};

async fn page(req: Request) -> Response {
    InertiaResponse::new("Page")
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}

fn redirect(to: &str) -> Response {
    Redirect::to(to).into()
}

fn router() -> Router {
    Router::new()
        .get("/page", page)
        .get("/hop", |_req: Request| async { redirect("/page") })
        .get("/flash-then-redirect", |_req: Request| async {
            App::flash("toast", "Saved");
            redirect("/hop")
        })
        .get("/flash-then-text", |_req: Request| async {
            App::flash("toast", "Queued");
            text("ok")
        })
        .get("/logout", |_req: Request| async {
            App::clear_history();
            redirect("/hop")
        })
        .get("/to-section", |_req: Request| async {
            let response: Response = Redirect::to("/hop").preserve_fragment().into();
            response
        })
        .get("/inertia-flash", |_req: Request| async {
            Inertia::flash("one", 1).map_err(HttpResponse::from)?;
            Inertia::flash(Toast::Success, "done").map_err(HttpResponse::from)?;
            Inertia::flash_many([("two", 2), ("three", 3)]).map_err(HttpResponse::from)?;
            redirect("/hop")
        })
        .get("/read-and-pull", |req: Request| async move {
            Inertia::flash("a", 1).map_err(HttpResponse::from)?;
            let got = Inertia::get_flashed(&req);
            let pulled = Inertia::pull_flashed(&req);
            let after = Inertia::get_flashed(&req);
            Ok(HttpResponse::json(
                json!({"got": got, "pulled": pulled, "after": after}),
            ))
        })
        .get("/builder-flash", |req: Request| async move {
            // A response built and dropped: its flash is in the session
            // already, as Laravel's `Response::flash` puts it there.
            let _unused = InertiaResponse::new("Page").flash("built", true);
            Ok(HttpResponse::json(
                json!({"seen": Inertia::get_flashed(&req)}),
            ))
        })
        .get("/facade-flags", |_req: Request| async {
            Inertia::clear_history();
            Inertia::preserve_fragment();
            redirect("/hop")
        })
        .into()
}

/// A real session middleware over a memory store, then the Inertia stack.
async fn app() -> (Client, Arc<MemoryStore>) {
    ensure_crypt();
    let store = Arc::new(MemoryStore::default());
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            session_config(),
            store.clone(),
        ))
        .append(InertiaHeadersMiddleware::new())
        .append(Inertia303Middleware::new());
    (Client::new(serve(router(), registry).await), store)
}

const INERTIA: &[(&str, &str)] = &[("X-Inertia", "true")];

#[tokio::test]
async fn inp_flash_before_two_redirects_reaches_the_page_after_them_once() {
    let (mut client, _store) = app().await;
    assert_eq!(
        client
            .send("GET", "/flash-then-redirect", INERTIA)
            .await
            .status,
        302
    );
    assert_eq!(client.send("GET", "/hop", INERTIA).await.status, 302);

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["flash"]["toast"], "Saved", "{page}");

    let next = client.send("GET", "/page", INERTIA).await.page();
    assert!(next.get("flash").is_none(), "pulled by the page: {next}");
}

#[tokio::test]
async fn inp_flash_on_a_request_that_answers_no_page_reaches_the_next_page() {
    // In the session it does not depend on the response of the request
    // that set it.
    let (mut client, _store) = app().await;
    assert_eq!(
        client.send("GET", "/flash-then-text", &[]).await.status,
        200
    );

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["flash"]["toast"], "Queued", "{page}");
}

#[tokio::test]
async fn inp_clear_history_before_two_redirects_reaches_the_page_once() {
    // A logout followed by two redirects must still rotate the client's
    // history key, or the back button restores private pages.
    let (mut client, _store) = app().await;
    client.send("GET", "/logout", INERTIA).await;
    client.send("GET", "/hop", INERTIA).await;

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["clearHistory"], true, "{page}");

    let next = client.send("GET", "/page", INERTIA).await.page();
    assert!(next.get("clearHistory").is_none(), "{next}");
}

#[tokio::test]
async fn inp_preserve_fragment_before_two_redirects_is_emitted_once() {
    let (mut client, _store) = app().await;
    client.send("GET", "/to-section", INERTIA).await;
    client.send("GET", "/hop", INERTIA).await;

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["preserveFragment"], true, "{page}");

    let next = client.send("GET", "/page", INERTIA).await.page();
    assert!(next.get("preserveFragment").is_none(), "{next}");
}

#[tokio::test]
async fn inp_inertia_flash_data_lives_in_the_session_under_its_key() {
    let (mut client, store) = app().await;
    client.send("GET", "/flash-then-text", &[]).await;

    let session = store.only_session();
    let stored: Option<serde_json::Value> = session.get("_flash.new.inertia.flash_data");
    assert_eq!(stored, Some(serde_json::json!({"toast": "Queued"})));
}

#[tokio::test]
async fn inp_inertia_flash_takes_a_key_an_enum_key_or_a_map() {
    let (mut client, _store) = app().await;
    client.send("GET", "/inertia-flash", INERTIA).await;
    client.send("GET", "/hop", INERTIA).await;

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(
        page["flash"],
        json!({"one": 1, "success": "done", "two": 2, "three": 3}),
        "{page}"
    );
}

#[tokio::test]
async fn inp_get_flashed_reads_what_pull_flashed_removes() {
    let (mut client, _store) = app().await;
    let reply = client.send("GET", "/read-and-pull", &[]).await;
    let body: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(body["got"], json!({"a": 1}));
    assert_eq!(body["pulled"], json!({"a": 1}));
    assert_eq!(body["after"], json!({}));

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert!(page.get("flash").is_none(), "pulled, so gone: {page}");
}

#[tokio::test]
async fn inp_the_builder_flash_lives_in_the_session() {
    let (mut client, _store) = app().await;
    let reply = client.send("GET", "/builder-flash", &[]).await;
    let body: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(body["seen"], json!({"built": true}));

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["flash"]["built"], true, "{page}");
}

#[tokio::test]
async fn inp_the_facade_sets_both_history_flags_until_a_page_emits_them() {
    let (mut client, _store) = app().await;
    client.send("GET", "/facade-flags", INERTIA).await;
    client.send("GET", "/hop", INERTIA).await;

    let page = client.send("GET", "/page", INERTIA).await.page();
    assert_eq!(page["clearHistory"], true, "{page}");
    assert_eq!(page["preserveFragment"], true, "{page}");
}
