//! PFX-004 and PFX-010: every `Location`, `X-Inertia-Location` and
//! `X-Inertia-Redirect` the framework emits for a root-relative path carries
//! the root exactly once, and the targets PFX-010 names are left alone.

use std::sync::Arc;

use suprnova::{
    AuthMiddleware, HttpResponse, InertiaResponse, MiddlewareRegistry, Redirect, Request, Response,
    Router, SessionConfig, SessionMiddleware,
};

use crate::support::{self, MemorySessionStore, PREFIX};

fn route_7() -> String {
    suprnova::route("invoices.show", &[("id", "7")]).expect("the invoices route")
}

/// What `/redirect/{case}` answers with.
fn redirect_for(case: &str, request: &Request) -> Response {
    match case {
        "plain" => Redirect::to("/dashboard").into(),
        "route" => Redirect::to(route_7()).into(),
        "named" => Redirect::route("invoices.show").with("id", "7").into(),
        "rooted-query" => Redirect::to("/billing?tab=2").into(),
        "rooted-fragment" => Redirect::to("/billing#x").into(),
        "query" => Redirect::to("?page=2").into(),
        "fragment" => Redirect::to("#top").into(),
        "relative" => Redirect::to("next").into(),
        "absolute" => Redirect::to("https://example.org/x").into(),
        "network" => Redirect::to("//cdn.example/x").into(),
        "helper" => suprnova::redirect_to("/x").into(),
        "bare" => suprnova::redirect().into(),
        "macro" => suprnova::redirect!("/x").into(),
        "signed" => Redirect::signed_route("invoices.show", &[("id", "7")])
            .map_err(|error| HttpResponse::text(error.to_string()).status(500))?
            .into(),
        "temporary" => {
            Redirect::temporary_signed_route("invoices.show", &[("id", "7")], 4_000_000_000)
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?
                .into()
        }
        "refresh-for" => Redirect::refresh_for(request).into(),
        "raw" => Ok(HttpResponse::new().status(302).header("Location", "/raw")),
        "inertia-location" => Ok(InertiaResponse::location(route_7())),
        "inertia-location-path" => Ok(InertiaResponse::location("/x")),
        "inertia-location-for" => Ok(InertiaResponse::location_for(request, "/x")),
        "inertia-redirect" => Ok(InertiaResponse::redirect("/x#f")),
        "version-conflict" => Ok(InertiaResponse::version_conflict("/x")),
        _ => Err(HttpResponse::text("unknown case").status(404)),
    }
}

fn router() -> Router {
    let router: Router = Router::new()
        .get("/invoices/{id}", |_request: Request| async {
            Ok(HttpResponse::text("invoice"))
        })
        .name("invoices.show");
    let router: Router = router
        .get("/redirect/{case}", |request: Request| async move {
            let case = request.param("case").unwrap_or_default().to_owned();
            redirect_for(&case, &request)
        })
        .get("/secret", |_request: Request| async {
            Ok(HttpResponse::text("secret"))
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.redirect("/old", "/new", 301)
}

/// The navigation header a case answers with.
async fn target(address: std::net::SocketAddr, case: &str, prefixed: bool) -> String {
    let path = format!("/redirect/{case}");
    let reply = if prefixed {
        support::get_prefixed(address, &path).await
    } else {
        support::get(address, &path, &[]).await
    };
    ["location", "x-inertia-location", "x-inertia-redirect"]
        .iter()
        .find_map(|name| reply.header(name))
        .unwrap_or_else(|| {
            panic!(
                "{case}: no navigation header ({}): {}",
                reply.status, reply.body
            )
        })
}

#[tokio::test]
async fn pfx_004_every_redirect_to_an_application_path_carries_the_root_once() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_004_every_redirect_to_an_application_path_carries_the_root_once",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;
    for (case, expected) in [
        ("plain", "/billing/dashboard"),
        ("route", "/billing/invoices/7"),
        ("named", "/billing/invoices/7"),
        ("helper", "/billing/x"),
        ("bare", "/billing/"),
        ("macro", "/billing/x"),
        ("refresh-for", "/billing/redirect/refresh-for"),
        ("raw", "/billing/raw"),
        ("inertia-location", "/billing/invoices/7"),
        ("inertia-location-path", "/billing/x"),
        ("inertia-location-for", "/billing/x"),
        ("inertia-redirect", "/billing/x#f"),
        ("version-conflict", "/billing/x"),
    ] {
        assert_eq!(target(address, case, true).await, expected, "{case}");
    }
    for case in ["signed", "temporary"] {
        let signed = target(address, case, true).await;
        assert!(
            signed.starts_with("/billing/invoices/7?") && !signed.contains("/billing/billing"),
            "{case}: {signed}"
        );
    }

    let old = support::get_prefixed(address, "/old").await;
    assert_eq!(old.header("location").as_deref(), Some("/billing/new"));

    let guarded = support::get_prefixed(address, "/secret").await;
    assert_eq!(guarded.status, 302);
    assert_eq!(
        guarded.header("location").as_deref(),
        Some("/billing/login")
    );
}

#[tokio::test]
async fn pfx_004_at_the_host_root_every_redirect_is_unchanged() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_004_at_the_host_root_every_redirect_is_unchanged",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;
    for (case, expected) in [
        ("plain", "/dashboard"),
        ("route", "/invoices/7"),
        ("bare", "/"),
        ("raw", "/raw"),
        ("inertia-redirect", "/x#f"),
    ] {
        assert_eq!(target(address, case, false).await, expected, "{case}");
    }
}

#[tokio::test]
async fn pfx_010_targets_under_the_root_and_non_paths_are_left_alone() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_010_targets_under_the_root_and_non_paths_are_left_alone",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;
    for (case, expected) in [
        ("rooted-query", "/billing?tab=2"),
        ("rooted-fragment", "/billing#x"),
        ("query", "?page=2"),
        ("fragment", "#top"),
        ("relative", "next"),
        ("absolute", "https://example.org/x"),
        ("network", "//cdn.example/x"),
    ] {
        assert_eq!(target(address, case, true).await, expected, "{case}");
    }
}

/// A router whose session records the previous and intended URLs.
fn session_router() -> Router {
    Router::new()
        .get("/page", |_request: Request| async {
            Ok(HttpResponse::text("page"))
        })
        .get("/back", |_request: Request| async {
            Redirect::back("/fallback").into()
        })
        .get("/members", |request: Request| async move {
            Redirect::guest(&request, "/login").into()
        })
        .get("/signed-in", |_request: Request| async {
            Redirect::intended("/home").into()
        })
        .get("/want", |_request: Request| async {
            Redirect::set_intended_url("/wanted");
            Ok(HttpResponse::text("noted"))
        })
        .into()
}

fn session_middleware() -> MiddlewareRegistry {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    MiddlewareRegistry::new().append(SessionMiddleware::with_store(
        config,
        Arc::new(MemorySessionStore::default()),
    ))
}

#[tokio::test]
async fn pfx_004_back_guest_and_intended_carry_the_root_once() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_004_back_guest_and_intended_carry_the_root_once",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(session_router(), session_middleware()).await;
    let prefix = ("x-forwarded-prefix", PREFIX);

    let page = support::get(address, "/page?tab=2", &[prefix]).await;
    let cookie = page.cookie_pair("suprnova_session");
    let with_cookie = [prefix, ("cookie", cookie.as_str())];

    let back = support::get(address, "/back", &with_cookie).await;
    assert_eq!(
        back.header("location").as_deref(),
        Some("/billing/page?tab=2"),
        "the previous URL is recorded and sent back with the root once"
    );

    let guest = support::get(address, "/members?x=1", &with_cookie).await;
    assert_eq!(guest.header("location").as_deref(), Some("/billing/login"));
    let intended = support::get(address, "/signed-in", &with_cookie).await;
    assert_eq!(
        intended.header("location").as_deref(),
        Some("/billing/members?x=1")
    );

    support::get(address, "/want", &with_cookie).await;
    let wanted = support::get(address, "/signed-in", &with_cookie).await;
    assert_eq!(
        wanted.header("location").as_deref(),
        Some("/billing/wanted")
    );
}
