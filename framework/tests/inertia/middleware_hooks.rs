//! PAR-054: the Inertia middleware's decisions can be replaced through a
//! hook trait, and the stack can be registered on a route group instead of
//! globally.
//!
//! Laravel's references are the overridable methods of `Inertia\Middleware`
//! (`version`, `share`, `shareOnce`, `rootView`, `urlResolver`,
//! `onEmptyResponse`, `onVersionChange`, `onRedirectWithFragment`) and its
//! registration on route groups in inertia-laravel 3.5.1.

use std::sync::Arc;

use suprnova::indexmap::IndexMap;
use suprnova::middleware::has_middleware_alias;
use suprnova::{
    DefaultInertiaHooks, HttpResponse, Inertia, InertiaConfig, InertiaMiddlewareHooks,
    InertiaRequestExt, InertiaResponse, InertiaVisit, MiddlewareRegistry, PageUrlResolver, Prop,
    Redirect, Request, Response, Router,
};

use crate::protocol_harness::{Client, serve};

/// Hooks that replace the three response decisions and add to the page.
struct AppHooks;

impl InertiaMiddlewareHooks for AppHooks {
    fn version(&self, _request: &dyn InertiaRequestExt) -> Option<String> {
        Some("v9".to_string())
    }

    fn share(&self, request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut props = IndexMap::new();
        props.insert(
            "path".to_string(),
            Prop::eager(serde_json::json!(request.path())),
        );
        props
    }

    fn share_once(&self, _request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut props = IndexMap::new();
        props.insert("plans".to_string(), Prop::eager(serde_json::json!(["pro"])));
        props
    }

    fn on_empty_response(&self, _visit: &InertiaVisit, _response: HttpResponse) -> HttpResponse {
        HttpResponse::new().status(204)
    }

    fn on_version_change(&self, visit: &InertiaVisit, _response: HttpResponse) -> HttpResponse {
        HttpResponse::text(format!("stale on {}", visit.path())).status(200)
    }

    fn on_redirect_with_fragment(
        &self,
        visit: &InertiaVisit,
        response: HttpResponse,
    ) -> HttpResponse {
        // Delegate to the framework's decision, then mark it.
        DefaultInertiaHooks
            .on_redirect_with_fragment(visit, response)
            .header("X-Hooked", "yes")
    }
}

/// Hooks whose empty-response answer is a redirect with a fragment.
struct FragmentOnEmpty;

impl InertiaMiddlewareHooks for FragmentOnEmpty {
    // The version the requests below carry, so the version check lets them
    // through to the handler.
    fn version(&self, _request: &dyn InertiaRequestExt) -> Option<String> {
        Some("v9".to_string())
    }

    fn on_empty_response(&self, _visit: &InertiaVisit, _response: HttpResponse) -> HttpResponse {
        HttpResponse::new()
            .status(302)
            .header("Location", "/app/page#top")
    }
}

/// Hooks that override nothing: every decision is the framework's.
struct Defaults;

impl InertiaMiddlewareHooks for Defaults {}

/// The routes behind the Inertia stack built from `config`, on a group.
async fn grouped(config: &InertiaConfig) -> Client {
    let router: Router = Router::new()
        .group("/app", |r| {
            r.get("/empty", |_req: Request| async {
                let response: Response = Ok(HttpResponse::new());
                response
            })
            .get("/fragment", |_req: Request| async {
                let response: Response = Redirect::to("/app/page#top").into();
                response
            })
            .get("/page", |req: Request| async move {
                InertiaResponse::new("Page")
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            })
            .put("/save", |_req: Request| async {
                let response: Response = Redirect::to("/app/page").into();
                response
            })
        })
        .middleware(Inertia::middleware(config))
        .into();
    let router: Router = router
        .group("/api", |r| {
            r.put("/save", |_req: Request| async {
                let response: Response = Redirect::to("/app/page").into();
                response
            })
        })
        .into();
    Client::new(serve(router, MiddlewareRegistry::new()).await)
}

const INERTIA: &[(&str, &str)] = &[("X-Inertia", "true"), ("X-Inertia-Version", "v9")];

#[tokio::test]
async fn inp_an_on_empty_response_hook_replaces_the_redirect_back() {
    let mut client = grouped(&InertiaConfig::new().hooks(AppHooks)).await;
    let reply = client.send("GET", "/app/empty", INERTIA).await;
    assert_eq!(reply.status, 204, "{reply:?}");
    assert_eq!(reply.header("location"), None);
}

#[tokio::test]
async fn inp_a_fragment_redirect_an_on_empty_response_hook_returns_is_converted_too() {
    // PAR-048's fragment rule reads the response that is sent, the hook's
    // included, not the empty 200 the handler returned.
    let mut client = grouped(&InertiaConfig::new().hooks(FragmentOnEmpty)).await;
    let reply = client.send("GET", "/app/empty", INERTIA).await;
    assert_eq!(reply.status, 409, "{reply:?}");
    assert_eq!(reply.header("x-inertia-redirect"), Some("/app/page#top"));
}

#[tokio::test]
async fn inp_an_on_version_change_hook_replaces_the_409() {
    let mut client = grouped(&InertiaConfig::new().hooks(AppHooks)).await;
    let reply = client
        .send(
            "GET",
            "/app/page",
            &[("X-Inertia", "true"), ("X-Inertia-Version", "v1")],
        )
        .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, "stale on /app/page");
}

#[tokio::test]
async fn inp_an_on_redirect_with_fragment_hook_sees_the_framework_answer() {
    let mut client = grouped(&InertiaConfig::new().hooks(AppHooks)).await;
    let reply = client.send("GET", "/app/fragment", INERTIA).await;
    assert_eq!(reply.status, 409);
    assert_eq!(reply.header("x-inertia-redirect"), Some("/app/page#top"));
    assert_eq!(reply.header("x-hooked"), Some("yes"));
}

#[tokio::test]
async fn inp_version_and_share_hooks_reach_the_page() {
    let mut client = grouped(&InertiaConfig::new().hooks(AppHooks)).await;
    // The hook's version is the one the client is compared against...
    let reply = client.send("GET", "/app/page", INERTIA).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let page = reply.page();
    // ...and the one the page carries.
    assert_eq!(page["version"], "v9");
    assert_eq!(page["props"]["path"], "/app/page");
    assert_eq!(page["props"]["plans"], serde_json::json!(["pro"]));
    assert!(
        page["onceProps"].get("plans").is_some(),
        "share_once props are once props: {page}"
    );
}

#[tokio::test]
async fn inp_hooks_that_override_nothing_keep_the_framework_behaviour() {
    let mut client = grouped(&InertiaConfig::new().hooks(Defaults).version("v9")).await;
    let reply = client.send("GET", "/app/empty", INERTIA).await;
    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/"));
    let reply = client
        .send(
            "GET",
            "/app/page",
            &[("X-Inertia", "true"), ("X-Inertia-Version", "v1")],
        )
        .await;
    assert_eq!(reply.status, 409);
}

// ---- the stack on a route group ----

#[tokio::test]
async fn inp_a_group_with_the_inertia_stack_has_vary_and_the_303() {
    let mut client = grouped(&InertiaConfig::new().version("v9")).await;
    let reply = client.send("PUT", "/app/save", INERTIA).await;
    assert_eq!(reply.status, 303);
    assert_eq!(reply.header("vary"), Some("X-Inertia"));
}

#[tokio::test]
async fn inp_a_group_without_the_inertia_stack_has_neither() {
    let mut client = grouped(&InertiaConfig::new().version("v9")).await;
    let reply = client.send("PUT", "/api/save", INERTIA).await;
    assert_eq!(reply.status, 302, "no Inertia conversion on the API group");
    assert_eq!(reply.header("vary"), None);
}

#[tokio::test]
async fn inp_install_off_the_global_list_registers_the_named_stack() {
    // A group names the stack instead of carrying a value; no global
    // middleware is registered, so a route outside such a group has
    // nothing of Inertia's.
    let before = suprnova::middleware::global_middleware_count();
    // DevTools off: a test of this binary may set `APP_ENV=local`, where
    // An opted-in DevTools answers its endpoints from a global
    // middleware this count is not about.
    Inertia::install(
        &InertiaConfig::new()
            .development(true)
            .version("v9")
            .register_globally(false)
            .devtools(suprnova::DevToolsConfig::new().enabled(false)),
    )
    .expect("dev-mode install needs no manifest");
    assert_eq!(suprnova::middleware::global_middleware_count(), before);
    assert!(has_middleware_alias("inertia"));

    let router: Router = Router::new()
        .group("/app", |r| {
            r.put("/save", |_req: Request| async {
                let response: Response = Redirect::to("/app/page").into();
                response
            })
        })
        .middleware_named("inertia")
        .into();
    let router: Router = router
        .group("/api", |r| {
            r.put("/save", |_req: Request| async {
                let response: Response = Redirect::to("/app/page").into();
                response
            })
        })
        .into();
    let mut client = Client::new(serve(router, MiddlewareRegistry::from_global()).await);

    let app = client.send("PUT", "/app/save", INERTIA).await;
    assert_eq!((app.status, app.header("vary")), (303, Some("X-Inertia")));
    let api = client.send("PUT", "/api/save", INERTIA).await;
    assert_eq!((api.status, api.header("vary")), (302, None));
}

/// Hooks that shape the document and the page `url`.
struct DocumentHooks;

impl InertiaMiddlewareHooks for DocumentHooks {
    fn root_view(&self, _request: &dyn InertiaRequestExt, config: InertiaConfig) -> InertiaConfig {
        config.default_title("Admin Console")
    }

    fn url_resolver(&self) -> Option<PageUrlResolver> {
        Some(Arc::new(|_request: &dyn InertiaRequestExt| {
            "/canonical".to_string()
        }))
    }
}

#[tokio::test]
async fn inp_root_view_and_url_resolver_hooks_shape_the_document_and_the_url() {
    let mut client = grouped(&InertiaConfig::new().hooks(DocumentHooks).version("v9")).await;
    let html = client.send("GET", "/app/page", &[]).await;
    assert!(
        html.body.contains("<title>Admin Console</title>"),
        "{}",
        html.body
    );
    let json = client.send("GET", "/app/page", INERTIA).await;
    let page = json.page();
    assert_eq!(page["url"], "/canonical", "{page}");
}
