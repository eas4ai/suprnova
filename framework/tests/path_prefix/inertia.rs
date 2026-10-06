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
    vite_tags_from(base, None, true).await
}

/// The Vite tags of a production page whose assets sit under `base`, read
/// from the Vite manifest at `manifest` when one is given (the fallback
/// tags otherwise), requested behind [`PREFIX`] when `prefixed`.
async fn vite_tags_from(
    base: &'static str,
    manifest: Option<std::path::PathBuf>,
    prefixed: bool,
) -> String {
    let router: Router = Router::new()
        .get("/shell", move |request: Request| {
            let manifest = manifest.clone();
            async move {
                let mut config = InertiaConfig::new()
                    .development(false)
                    .entry_point("src/main.ts")
                    .assets_base_url(base);
                if let Some(manifest) = manifest {
                    config = config.manifest_path(manifest);
                }
                InertiaResponse::new("Page")
                    .with_config(config)
                    .resolve(&request)
                    .await
                    .map_err(HttpResponse::from)
            }
        })
        .into();
    let address = support::serve(router, MiddlewareRegistry::new()).await;
    let reply = if prefixed {
        support::get_prefixed(address, "/shell").await
    } else {
        support::get(address, "/shell", &[]).await
    };
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply
        .body
        .lines()
        .filter(|line| line.contains("main") && (line.contains(".js") || line.contains(".css")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn pfx_005_an_assets_base_url_of_slash_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_005_an_assets_base_url_of_slash_carries_the_root",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");

    let fallback = vite_tags_from("/", None, true).await;
    assert!(fallback.contains(r#"src="/billing/main.js""#), "{fallback}");
    assert!(
        fallback.contains(r#"href="/billing/main.css""#),
        "{fallback}"
    );

    let dir = tempfile::tempdir().expect("a manifest directory");
    let manifest = dir.path().join("manifest.json");
    std::fs::write(
        &manifest,
        r#"{"src/main.ts":{"file":"main-1a2b.js","css":["main-3c4d.css"],"isEntry":true}}"#,
    )
    .expect("write the manifest");
    let built = vite_tags_from("/", Some(manifest.clone()), true).await;
    assert!(built.contains(r#"src="/billing/main-1a2b.js""#), "{built}");
    assert!(
        built.contains(r#"href="/billing/main-3c4d.css""#),
        "{built}"
    );

    // At the host root the tags are what they were before the root.
    let host_fallback = vite_tags_from("/", None, false).await;
    assert!(
        host_fallback.contains(r#"src="/main.js""#),
        "{host_fallback}"
    );
    let host_built = vite_tags_from("/", Some(manifest), false).await;
    assert!(
        host_built.contains(r#"src="/main-1a2b.js""#),
        "{host_built}"
    );
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
