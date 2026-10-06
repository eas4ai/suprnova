//! PFX-005 and PFX-012: the Inertia page URL, the version 409, the
//! validation redirect, the Vite tags, and the shared `root` prop.

use std::sync::Arc;

use serde_json::json;
use suprnova::{
    App, HttpResponse, InertiaConfig, InertiaResponse, InertiaValidationRedirectMiddleware,
    InertiaVersionMiddleware, LocaleShare, MiddlewareRegistry, Request, RootShare, Router,
};

use crate::support::{self, PREFIX};

fn page_router() -> Router {
    Router::new()
        .get("/page", |request: Request| async move {
            InertiaResponse::new("Page")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/versioned", |request: Request| async move {
            InertiaResponse::new("Page")
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .middleware(InertiaVersionMiddleware::new("v2"))
        .into()
}

#[tokio::test]
async fn pfx_005_the_page_url_and_the_version_409_carry_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_005_the_page_url_and_the_version_409_carry_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let address = support::serve(page_router(), MiddlewareRegistry::new()).await;
    let inertia = [("x-forwarded-prefix", PREFIX), ("x-inertia", "true")];

    let page = support::get(address, "/page?sort=name", &inertia).await;
    assert_eq!(page.status, 200, "{}", page.body);
    assert_eq!(page.json()["url"], "/billing/page?sort=name");

    let conflict = support::get(
        address,
        "/versioned?sort=name",
        &[
            ("x-forwarded-prefix", PREFIX),
            ("x-inertia", "true"),
            ("x-inertia-version", "v1"),
        ],
    )
    .await;
    assert_eq!(conflict.status, 409);
    assert_eq!(
        conflict.header("x-inertia-location").as_deref(),
        Some("/billing/versioned?sort=name")
    );

    let at_host_root = support::get(address, "/page?sort=name", &[("x-inertia", "true")]).await;
    assert_eq!(at_host_root.json()["url"], "/page?sort=name");
}

fn validation_router() -> Router {
    Router::new()
        .post("/submit", |_request: Request| async {
            Err(HttpResponse::json(json!({
                "message": "The given data was invalid.",
                "errors": {"email": ["The email field is required."]},
            }))
            .status(422))
        })
        .middleware(InertiaValidationRedirectMiddleware::new())
        .into()
}

async fn validation_target(address: std::net::SocketAddr, headers: &[(&str, &str)]) -> String {
    let mut all: Vec<(&str, &[u8])> = vec![("x-inertia", b"true")];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| (*name, value.as_bytes())),
    );
    let reply = support::send(address, "POST", "/submit", &all, b"").await;
    assert_eq!(reply.status, 303, "{}", reply.body);
    reply.header("location").expect("a Location")
}

#[tokio::test]
async fn pfx_005_the_validation_redirect_follows_only_a_referer_under_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_005_the_validation_redirect_follows_only_a_referer_under_the_root",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(validation_router(), MiddlewareRegistry::new()).await;
    let prefix = ("x-forwarded-prefix", PREFIX);

    // A Referer under the root keeps its root, once.
    assert_eq!(
        validation_target(
            address,
            &[prefix, ("referer", "http://app.test/billing/form?x=1")]
        )
        .await,
        "/billing/form?x=1"
    );
    // A Referer on the same host outside the root is foreign: the fallback
    // is the request's own URL, root included.
    assert_eq!(
        validation_target(
            address,
            &[prefix, ("referer", "http://app.test/other/form")]
        )
        .await,
        "/billing/submit"
    );
    assert_eq!(
        validation_target(address, &[prefix, ("referer", "/other/form")]).await,
        "/billing/submit"
    );
    // At the host root, nothing changes.
    assert_eq!(
        validation_target(address, &[("referer", "http://app.test/form")]).await,
        "/form"
    );
}

#[tokio::test]
async fn pfx_005_the_validation_redirect_compares_the_forwarded_host() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_005_the_validation_redirect_compares_the_forwarded_host",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(validation_router(), MiddlewareRegistry::new()).await;
    let forwarded = [
        ("x-forwarded-prefix", PREFIX),
        ("x-forwarded-host", "public.example"),
    ];

    let mut raw_host = forwarded.to_vec();
    raw_host.push(("referer", "http://app.test/billing/form"));
    assert_eq!(
        validation_target(address, &raw_host).await,
        "/billing/submit",
        "a Referer matching only the raw Host was followed"
    );

    let mut public = forwarded.to_vec();
    public.push(("referer", "http://public.example/billing/form"));
    assert_eq!(
        validation_target(address, &public).await,
        "/billing/form",
        "a Referer matching the forwarded host was treated as foreign"
    );
}

/// The Vite tags of a production page whose assets sit under `base`.
async fn vite_tags(base: &'static str) -> String {
    let router: Router = Router::new()
        .get("/shell", move |request: Request| async move {
            InertiaResponse::new("Page")
                .with_config(
                    InertiaConfig::new()
                        .development(false)
                        .assets_base_url(base),
                )
                .resolve(&request)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;
    let reply = support::get_prefixed(address, "/shell").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply
        .body
        .lines()
        .filter(|line| line.contains("main.js") || line.contains("main.css"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn pfx_005_the_vite_tags_carry_the_root_escaped_and_never_on_a_cdn() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_005_the_vite_tags_carry_the_root_escaped_and_never_on_a_cdn",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");

    let local = vite_tags("/assets").await;
    assert!(
        local.contains(r#"src="/billing/assets/main.js""#),
        "{local}"
    );
    assert!(
        local.contains(r#"href="/billing/assets/main.css""#),
        "{local}"
    );

    let cdn = vite_tags("https://cdn.example/assets").await;
    assert!(
        cdn.contains(r#"src="https://cdn.example/assets/main.js""#),
        "{cdn}"
    );
    assert!(!cdn.contains("/billing"), "{cdn}");

    let network = vite_tags("//cdn.example/assets").await;
    assert!(
        network.contains(r#"src="//cdn.example/assets/main.js""#),
        "{network}"
    );

    let quoted = vite_tags("/as\"sets").await;
    assert!(
        quoted.contains(r#"src="/billing/as&quot;sets/main.js""#),
        "an attribute value was written unescaped: {quoted}"
    );
}

#[tokio::test]
async fn pfx_012_root_share_gives_every_page_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_012_root_share_gives_every_page_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    App::register_inertia_shared(Arc::new(RootShare::around(Arc::new(LocaleShare))));
    let address = support::serve(page_router(), MiddlewareRegistry::new()).await;

    let behind = support::get(
        address,
        "/page",
        &[("x-forwarded-prefix", PREFIX), ("x-inertia", "true")],
    )
    .await;
    let props = &behind.json()["props"];
    assert_eq!(props["root"], "/billing");
    assert!(
        props["lang"].is_object(),
        "the carried provider's prop: {props}"
    );

    let at_host_root = support::get(address, "/page", &[("x-inertia", "true")]).await;
    assert_eq!(at_host_root.json()["props"]["root"], "");
}
