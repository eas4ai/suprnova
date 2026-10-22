//! A shared page shell never retains a visitor's values or slot dependencies.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};
use suprnova::{
    Auth, FrameworkError, HttpResponse, Inertia, InertiaConfig, InertiaProps, InertiaRequestExt,
    InertiaResponse, Middleware, Model, Next, Prop, Request, Response, Router, SsrConfig,
    SsrGateway, SsrResponse, attrs,
};

use crate::render_cache_middleware_support::{self as support, Post, User, dispatch_get};

static PUBLIC_RUNS: AtomicUsize = AtomicUsize::new(0);
static PRIVATE_RUNS: AtomicUsize = AtomicUsize::new(0);
static SHARED_RUNS: AtomicUsize = AtomicUsize::new(0);
static SSR_RUNS: AtomicUsize = AtomicUsize::new(0);

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct PageProps {
    #[inertia(public)]
    article: Prop,
    #[inertia(per_request)]
    viewer: Prop,
}

struct SessionAndGuard;

#[suprnova::async_trait]
impl Middleware for SessionAndGuard {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("x-test-deny") == Some("yes") {
            return Err(HttpResponse::text("denied").status(403));
        }
        let id = request
            .header("x-test-session")
            .unwrap_or("anonymous")
            .to_owned();
        if id == "without-session" {
            return next(request).await;
        }
        let mut session = suprnova::session::SessionData::new(id.clone(), format!("csrf-{id}"));
        session.put("visitor", id.clone());
        session.put(
            "_flash.old.errors.default",
            json!({"field": [format!("error-{id}")]}),
        );
        session.put(
            "_flash.old.inertia.flash_data",
            json!({"notice": format!("flash-{id}")}),
        );
        let slot = Arc::new(Mutex::new(Some(session)));
        suprnova::session::session_scope_for_test(slot, next(request)).await
    }
}

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct EagerPageProps {
    #[inertia(public)]
    article: Prop,
    #[inertia(per_request)]
    viewer: CurrentViewer,
}

struct CurrentViewer;
impl serde::Serialize for CurrentViewer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PRIVATE_RUNS.fetch_add(1, Ordering::SeqCst);
        let id = Auth::id();
        let session = suprnova::session::session().and_then(|s| s.get::<String>("visitor"));
        serde::Serialize::serialize(&json!({"id": id, "session": session}), serializer)
    }
}

async fn page(request: Request) -> Response {
    let article = Prop::from_resolver(Arc::new(|| {
        Box::pin(async {
            PUBLIC_RUNS.fetch_add(1, Ordering::SeqCst);
            let article = Post::find(1)
                .await?
                .ok_or_else(|| FrameworkError::internal("missing article"))?;
            Ok(json!({"title": article.title}))
        })
    }));
    let large = request.header("x-test-large-viewer") == Some("yes");
    let builder = InertiaResponse::new("Cache/Stitched");
    let response = if request.header("x-test-eager-viewer") == Some("yes") {
        builder.cacheable(EagerPageProps {
            article,
            viewer: CurrentViewer,
        })
    } else {
        builder.cacheable(PageProps {
            article,
            viewer: Prop::from_resolver(Arc::new(move || {
                Box::pin(async move {
                    PRIVATE_RUNS.fetch_add(1, Ordering::SeqCst);
                    let id = Auth::id().unwrap_or_default();
                    let user = User::find(id.parse::<i64>().unwrap_or(0)).await?;
                    let session =
                        suprnova::session::session().and_then(|s| s.get::<String>("visitor"));
                    let padding = if large {
                        "x".repeat(suprnova_live::render_cache::entry::EntryLimits::default().max_body_bytes)
                    } else {
                        String::new()
                    };
                    Ok(json!({"name": user.map(|u| u.name), "session": session, "padding": padding}))
                })
            })),
        })
    }
    .map_err(HttpResponse::from)?;
    let response = response
        .resolve(&request)
        .await
        .map_err(HttpResponse::from)?;
    if request.header("x-test-protected-headers") == Some("yes") {
        Ok(response.with_headers(PROTECTED_HEADERS))
    } else {
        Ok(response)
    }
}

const PROTECTED_HEADERS: [(&str, &str); 6] = [
    ("Content-Disposition", "attachment; filename=résumé.json"),
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Embedder-Policy", "require-corp"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
    ("Permissions-Policy", "camera=()"),
    ("X-Frame-Options", "DENY"),
];

async fn boot(ssr: bool) -> Arc<support::Harness> {
    boot_with_nonce(ssr, false).await
}

async fn boot_with_nonce(ssr: bool, nonce: bool) -> Arc<support::Harness> {
    boot_with_shared_boundary(ssr, nonce, false).await
}

async fn boot_with_shared_boundary(
    ssr: bool,
    nonce: bool,
    public_errors: bool,
) -> Arc<support::Harness> {
    PUBLIC_RUNS.store(0, Ordering::SeqCst);
    PRIVATE_RUNS.store(0, Ordering::SeqCst);
    SSR_RUNS.store(0, Ordering::SeqCst);
    SHARED_RUNS.store(0, Ordering::SeqCst);
    let router: Router = Router::new()
        .get("/inertia-stitched", page)
        .middleware(SessionAndGuard)
        .into();
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).unwrap())
        .vary(VarianceDimension::Locale)
        .build()
        .unwrap();
    let router = router
        .try_render_cache("/inertia-stitched", policy)
        .unwrap();
    let harness = support::boot_with_inertia_routes_setup(router, || {
        let mut config = InertiaConfig::new().development(true).version("cache-v1");
        if ssr {
            config = config.ssr("http://unused").ssr_ensure_bundle_exists(false);
        } else {
            config = config.ssr_disabled();
        }
        Inertia::install(&config).unwrap();
        if nonce {
            suprnova::middleware::register_global_middleware(suprnova::SecurityHeaders::strict());
        }
        suprnova::App::inertia_share_lazy("auth", || async {
            SHARED_RUNS.fetch_add(1, Ordering::SeqCst);
            Ok::<_, FrameworkError>(json!({"id": Auth::id()}))
        });
        Inertia::cache_page::<PageProps>(
            "/inertia-stitched",
            "Cache/Stitched",
            if public_errors {
                &["errors", "flash"]
            } else {
                &[]
            },
            if public_errors {
                &["auth"]
            } else {
                &["auth", "errors", "flash"]
            },
        )
        .unwrap();
        suprnova::App::bind::<dyn SsrGateway>(Arc::new(EchoSsr));
    })
    .await;
    Post::create(attrs! { title: "shared article" })
        .await
        .unwrap();
    User::create(attrs! { name: "Alice" }).await.unwrap();
    User::create(attrs! { name: "Bob" }).await.unwrap();
    harness
}

fn headers(user: &'static str, session: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("X-Inertia", "true"),
        ("X-Inertia-Version", "cache-v1"),
        ("x-test-login", user),
        ("x-test-session", session),
    ]
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_two_users_and_sessions_share_only_public_props() {
    let h = boot(false).await;
    for (user, session, name) in [
        ("1", "session-a", "Alice"),
        ("2", "session-b", "Bob"),
        ("1", "session-c", "Alice"),
    ] {
        let response = dispatch_get(&h, "/inertia-stitched", &headers(user, session)).await;
        assert_eq!(response.status.as_u16(), 200);
        let page: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            page["props"]["viewer"],
            json!({"name": name, "session": session, "padding": ""})
        );
        assert_eq!(page["props"]["article"]["title"], "shared article");
        assert_eq!(page["props"]["auth"]["id"], user);
        assert_eq!(page["props"]["errors"]["field"], format!("error-{session}"));
        assert_eq!(page["flash"]["notice"], format!("flash-{session}"));
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
    }
    assert_eq!(
        PUBLIC_RUNS.load(Ordering::SeqCst),
        1,
        "the public resolver ran only on the miss"
    );
    assert_eq!(PRIVATE_RUNS.load(Ordering::SeqCst), 3);
    assert_eq!(SHARED_RUNS.load(Ordering::SeqCst), 3);
    assert_eq!(SSR_RUNS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_guard_refusal_wins_without_resolving_a_prop() {
    let h = boot(false).await;
    dispatch_get(&h, "/inertia-stitched", &headers("1", "session-a")).await;
    let mut denied = headers("2", "session-b");
    denied.push(("x-test-deny", "yes"));
    let response = dispatch_get(&h, "/inertia-stitched", &denied).await;
    assert_eq!(response.status.as_u16(), 403);
    assert_eq!(&response.body[..], b"denied");
    assert_eq!(PRIVATE_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(SHARED_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_slot_write_keeps_shell_and_shell_write_rebuilds_it() {
    let h = boot(false).await;
    dispatch_get(&h, "/inertia-stitched", &headers("1", "a")).await;
    support::rename_user(&h, 1, "Alice changed").await;
    let response = dispatch_get(&h, "/inertia-stitched", &headers("1", "b")).await;
    let page: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(page["props"]["viewer"]["name"], "Alice changed");
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
    let mut article = Post::find(1).await.unwrap().unwrap();
    article.title = "updated article".to_owned();
    article.save().await.unwrap();
    let response = dispatch_get(&h, "/inertia-stitched", &headers("2", "c")).await;
    let page: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(page["props"]["article"]["title"], "updated article");
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 2);
}

struct EchoSsr;

#[suprnova::async_trait]
impl SsrGateway for EchoSsr {
    async fn dispatch(
        &self,
        config: &SsrConfig,
        _: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        if !config.enabled {
            return Ok(None);
        }
        SSR_RUNS.fetch_add(1, Ordering::SeqCst);
        let text = page["props"]["viewer"]["name"].as_str().unwrap_or("");
        Ok(Some(SsrResponse {
            head: vec![],
            body: format!(
                "<div id=\"app\">{text}</div><script type=\"application/json\" data-page=\"app\">{}</script>",
                serde_json::to_string(page)
                    .unwrap()
                    .replace('<', "\\u003c")
                    .replace('>', "\\u003e")
            ),
        }))
    }
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_ssr_and_hydration_json_agree_on_every_document_hit() {
    let h = boot(true).await;
    for (user, name) in [("1", "Alice"), ("2", "Bob")] {
        let response = dispatch_get(
            &h,
            "/inertia-stitched",
            &[("x-test-login", user), ("x-test-session", user)],
        )
        .await;
        assert_eq!(response.status.as_u16(), 200);
        let html = String::from_utf8_lossy(&response.body);
        assert!(
            html.contains(&format!("<div id=\"app\">{name}</div>")),
            "{html}"
        );
        let json = html
            .split("data-page=\"app\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let page: Value = serde_json::from_str(json).unwrap();
        assert_eq!(page["props"]["viewer"]["name"], name);
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
    }
    assert_eq!(SSR_RUNS.load(Ordering::SeqCst), 2);
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_nonce_policy_renders_without_reusing_an_inertia_shell() {
    let h = boot_with_nonce(false, true).await;
    let first = dispatch_get(&h, "/inertia-stitched", &[("x-test-login", "1")]).await;
    let second = dispatch_get(&h, "/inertia-stitched", &[("x-test-login", "2")]).await;
    assert_eq!(first.status.as_u16(), 200);
    assert_eq!(second.status.as_u16(), 200);
    assert_ne!(
        first.header("content-security-policy"),
        second.header("content-security-policy")
    );
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 2);
    assert_eq!(PRIVATE_RUNS.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_route_local_eager_hook_is_refused_at_boot_without_running() {
    static HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);
    struct EagerShares;
    impl suprnova::InertiaMiddlewareHooks for EagerShares {
        fn share(&self, _: &dyn InertiaRequestExt) -> suprnova::indexmap::IndexMap<String, Prop> {
            HOOK_RUNS.fetch_add(1, Ordering::SeqCst);
            [("viewer".to_owned(), Prop::eager("private".into()))].into()
        }
    }
    let _harness = support::boot_with_render_cache().await;
    Inertia::install(&InertiaConfig::new().development(true)).unwrap();
    Inertia::cache_page::<PageProps>(
        "/guarded",
        "Cache/Stitched",
        &[],
        &["errors", "flash", "viewer"],
    )
    .unwrap();
    let local = InertiaConfig::new().hooks(EagerShares);
    let router: Router = Router::new()
        .get("/guarded", page)
        .middleware(SessionAndGuard)
        .middleware(Inertia::middleware(&local))
        .into();
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).unwrap())
        .build()
        .unwrap();
    let router = router.try_render_cache("/guarded", policy).unwrap();
    let mut config = suprnova::render_cache::config::RenderCacheConfig::from_env().unwrap();
    config.enabled = true;
    let error = match suprnova::render_cache::RenderCache::install(router, config).await {
        Err(error) => error,
        Ok(_) => panic!("route-local opaque shares must fail boot"),
    };
    assert!(error.to_string().contains("/guarded"));
    assert!(error.to_string().contains("middleware hook"));
    assert_eq!(HOOK_RUNS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_stitched_hit_replays_protected_headers_and_declared_vary() {
    let harness = boot(false).await;
    let mut first_headers = headers("1", "alice");
    first_headers.push(("x-test-protected-headers", "yes"));
    let first = dispatch_get(&harness, "/inertia-stitched", &first_headers).await;
    let second = dispatch_get(&harness, "/inertia-stitched", &headers("2", "bob")).await;
    for (name, value) in PROTECTED_HEADERS {
        assert_eq!(first.header(name), Some(value));
        assert_eq!(second.header(name), Some(value));
    }
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
    assert!(second.header("age").is_some());
    let vary = second.header("vary").unwrap().to_ascii_lowercase();
    for name in ["x-inertia", "x-inertia-partial-data", "x-inertia-version"] {
        assert!(vary.split(',').any(|part| part.trim() == name), "{vary}");
    }
    assert_eq!(second.header("cache-control"), Some("private, no-store"));
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_public_shared_declaration_cannot_hide_session_reads() {
    let harness = boot_with_shared_boundary(false, false, true).await;
    let anonymous = dispatch_get(
        &harness,
        "/inertia-stitched",
        &headers("1", "without-session"),
    )
    .await;
    assert!(
        anonymous.header("age").is_some(),
        "session-free shell publishes"
    );
    let first = dispatch_get(&harness, "/inertia-stitched", &headers("1", "alice")).await;
    let second = dispatch_get(&harness, "/inertia-stitched", &headers("2", "bob")).await;
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    let second_page: Value = serde_json::from_slice(&second.body).unwrap();
    assert_eq!(first["props"]["errors"]["field"], "error-alice");
    assert_eq!(second_page["props"]["errors"]["field"], "error-bob");
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 3);
    assert!(second.header("age").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_eager_request_serializers_keep_their_reads_out_of_the_shell() {
    let harness = boot(false).await;
    for (user, session) in [("1", "alice"), ("2", "bob")] {
        let mut headers = headers(user, session);
        headers.push(("x-test-eager-viewer", "yes"));
        let response = dispatch_get(&harness, "/inertia-stitched", &headers).await;
        let page: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(page["props"]["viewer"]["id"], user);
        assert_eq!(page["props"]["viewer"]["session"], session);
    }
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(PRIVATE_RUNS.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[serial_test::serial]
async fn cache_012_large_request_prop_uses_current_values_without_repeating_work() {
    let harness = boot(false).await;
    let first = dispatch_get(&harness, "/inertia-stitched", &headers("1", "alice")).await;
    assert_eq!(first.status.as_u16(), 200);
    let mut large = headers("2", "bob");
    large.push(("x-test-large-viewer", "yes"));
    let response = dispatch_get(&harness, "/inertia-stitched", &large).await;
    assert_eq!(response.status.as_u16(), 200);
    let page: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(page["props"]["article"]["title"], "shared article");
    assert_eq!(page["props"]["viewer"]["name"], "Bob");
    assert_eq!(page["props"]["viewer"]["session"], "bob");
    assert_eq!(
        page["props"]["viewer"]["padding"].as_str().unwrap().len(),
        suprnova_live::render_cache::entry::EntryLimits::default().max_body_bytes
    );
    assert_eq!(response.header("cache-control"), Some("private, no-store"));
    assert_eq!(PUBLIC_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(PRIVATE_RUNS.load(Ordering::SeqCst), 2);
    assert_eq!(SHARED_RUNS.load(Ordering::SeqCst), 2);
}
