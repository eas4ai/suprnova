//! PFX-002 and PFX-011: one public root for the URLs the framework builds,
//! and `url::root()` for the ones a view writes.

use askama::Template;
use suprnova::{
    HttpResponse, InertiaErrorPageMiddleware, MiddlewareRegistry, Request, Router, url,
};

use crate::support::{self, PREFIX};

/// A route that answers with every URL builder PFX-002 names, one per line.
fn builders_router() -> Router {
    Router::new()
        .get("/invoices", |request: Request| async move {
            Ok(HttpResponse::text(
                [
                    url::to("/x"),
                    url::secure("/x"),
                    url::full(&request),
                    url::current(&request),
                    request.url(),
                    request.full_url(),
                    url::root(),
                ]
                .join("\n"),
            ))
        })
        .into()
}

async fn builders(app_url: &str, headers: &[(&str, &str)]) -> Vec<String> {
    support::install(app_url);
    let address = support::serve(builders_router(), MiddlewareRegistry::new()).await;
    let reply = support::get(address, "/invoices?page=2", headers).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.body.split('\n').map(str::to_owned).collect()
}

#[tokio::test]
async fn pfx_002_the_app_url_path_is_the_root_when_no_prefix_arrives() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_002_the_app_url_path_is_the_root_when_no_prefix_arrives",
    )
    .await
    {
        return;
    }
    let urls = builders("https://example.org/billing", &[]).await;
    assert_eq!(
        urls,
        [
            "https://example.org/billing/x",
            "https://example.org/billing/x",
            "https://example.org/billing/invoices?page=2",
            "/billing/invoices?page=2",
            "http://app.test/billing/invoices",
            "http://app.test/billing/invoices?page=2",
            "/billing",
        ]
    );
}

#[tokio::test]
async fn pfx_002_a_trusted_prefix_replaces_the_app_url_path_once() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_002_a_trusted_prefix_replaces_the_app_url_path_once",
    )
    .await
    {
        return;
    }
    // The APP_URL path and the header are both `/billing`: the root is
    // carried once, not twice.
    let urls = builders(
        "https://example.org/billing",
        &[("x-forwarded-prefix", PREFIX)],
    )
    .await;
    assert_eq!(
        urls,
        [
            "https://example.org/billing/x",
            "https://example.org/billing/x",
            "https://example.org/billing/invoices?page=2",
            "/billing/invoices?page=2",
            "http://app.test/billing/invoices",
            "http://app.test/billing/invoices?page=2",
            "/billing",
        ]
    );
}

#[tokio::test]
async fn pfx_002_a_trusted_prefix_wins_over_another_app_url_path() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_002_a_trusted_prefix_wins_over_another_app_url_path",
    )
    .await
    {
        return;
    }
    let urls = builders(
        "https://example.org/other",
        &[("x-forwarded-prefix", PREFIX)],
    )
    .await;
    assert_eq!(urls[0], "https://example.org/billing/x");
    assert_eq!(urls[3], "/billing/invoices?page=2");
}

#[tokio::test]
async fn pfx_002_at_the_host_root_every_url_is_unchanged() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_002_at_the_host_root_every_url_is_unchanged",
    )
    .await
    {
        return;
    }
    let urls = builders("https://example.org", &[]).await;
    assert_eq!(
        urls,
        [
            "https://example.org/x",
            "https://example.org/x",
            "https://example.org/invoices?page=2",
            "/invoices?page=2",
            "http://app.test/invoices",
            "http://app.test/invoices?page=2",
            "",
        ]
    );
}

#[test]
fn pfx_002_outside_a_request_the_root_is_the_app_url_path() {
    if !crate::own_process::is_child() {
        crate::own_process::run_alone(
            "root::pfx_002_outside_a_request_the_root_is_the_app_url_path",
        );
        return;
    }
    // A console command, a job, a mail: no request, so the root is the
    // APP_URL path.
    support::install("https://example.org/billing/");
    assert_eq!(url::root(), "/billing");
    assert_eq!(url::to("/invoices"), "https://example.org/billing/invoices");
    assert_eq!(url::to("invoices"), "https://example.org/billing/invoices");
    assert_eq!(
        url::to("/billing/invoices"),
        "https://example.org/billing/invoices"
    );
}

#[tokio::test]
async fn pfx_002_an_error_page_renders_under_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_002_an_error_page_renders_under_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let router: Router = Router::new()
        .get("/broken", |_request: Request| async {
            Err(HttpResponse::new().status(500))
        })
        .middleware(InertiaErrorPageMiddleware::new("Error"))
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;
    let reply = support::get(
        address,
        "/broken?step=1",
        &[("x-forwarded-prefix", PREFIX), ("x-inertia", "true")],
    )
    .await;
    assert_eq!(reply.status, 500, "{}", reply.body);
    let page = reply.json();
    assert_eq!(page["component"], "Error");
    assert_eq!(page["url"], "/billing/broken?step=1");
}

#[tokio::test]
async fn pfx_011_url_root_returns_the_root_without_a_trailing_slash() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_011_url_root_returns_the_root_without_a_trailing_slash",
    )
    .await
    {
        return;
    }
    let behind = builders("http://localhost", &[("x-forwarded-prefix", PREFIX)]).await;
    assert_eq!(behind[6], "/billing");
    let at_host_root = builders("http://localhost", &[]).await;
    assert_eq!(at_host_root[6], "");
    let from_app_url = builders("https://example.org/shop/", &[]).await;
    assert_eq!(from_app_url[6], "/shop");
}

/// The shipped `header-bar` view, called without a `brand_href`.
#[derive(Template)]
#[template(
    source = r#"{% import "header-bar/header-bar.html" as header %}{% call header::header_bar("site", "Brand") %}{% endcall %}"#,
    ext = "html",
    config = "tests/path_prefix/askama.toml"
)]
struct HeaderBarDefault;

#[tokio::test]
async fn pfx_011_the_header_bar_default_brand_link_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_011_the_header_bar_default_brand_link_carries_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let router: Router = Router::new()
        .get("/nav", |_request: Request| async {
            let html = HeaderBarDefault
                .render()
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            Ok(HttpResponse::html(html))
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;

    let behind = support::get_prefixed(address, "/nav").await;
    assert!(
        behind
            .body
            .contains(r#"<a class="sn-header-brand" href="/billing/">Brand</a>"#),
        "{}",
        behind.body
    );
    let at_host_root = support::get(address, "/nav", &[]).await;
    assert!(
        at_host_root
            .body
            .contains(r#"<a class="sn-header-brand" href="/">Brand</a>"#),
        "{}",
        at_host_root.body
    );
}
