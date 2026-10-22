//! Publication refuses private data before either representation reaches storage.

use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::{
    FrameworkError, HttpResponse, Inertia, InertiaProps, InertiaResponse, Request, Response,
    Router, SecurityHeaders, Storage,
};

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct PublicProps {
    #[inertia(public)]
    title: String,
}

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct PrivateProps {
    #[inertia(per_request)]
    viewer: String,
}

async fn page(req: Request) -> Response {
    InertiaResponse::new("Public")
        .with_config(
            suprnova::InertiaConfig::new()
                .version("pub-test-v1")
                .development(false)
                .ssr_disabled(),
        )
        .cacheable(PublicProps {
            title: "Public title".to_owned(),
        })
        .map_err(HttpResponse::from)?
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}

async fn session_page(req: Request) -> Response {
    let _ = suprnova::session::session();
    page(req).await
}

fn runtime(router: &Router) -> Result<std::sync::Arc<StaticPublication>, FrameworkError> {
    Storage::register_memory("pub-eligibility");
    StaticPublication::install(
        router,
        Publisher::new("pub-eligibility", "pub-test-v1", PublishConfig::default())?,
    )
}

fn marked() -> Result<Router, FrameworkError> {
    let router: Router = Router::new().get("/public", page).into();
    router.public_page::<PublicProps>("/public", "Public")
}

fn message<T>(result: Result<T, FrameworkError>) -> Result<String, FrameworkError> {
    result
        .err()
        .map(|error| error.to_string())
        .ok_or_else(|| FrameworkError::internal("expected publication refusal"))
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_unmarked_route_cannot_publish_even_with_typed_declaration()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new().get("/public", page).into();
    Inertia::public_page::<PublicProps>("/public", "Public")?;
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("public"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_per_request_props_refuse_without_stopping_install() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new().get("/public", page).into();
    let router = router.public_page::<PrivateProps>("/public", "Public")?;
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("per-request"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_handler_session_read_refuses_with_route_and_cause() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new().get("/public", session_page).into();
    let router = router.public_page::<PublicProps>("/public", "Public")?;
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("session"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_unlisted_shared_prop_refuses_with_route_and_cause() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    Inertia::share("settings", "private")?;
    let error = message(runtime(&marked()?)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("settings"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_nonce_csp_refuses_with_route_and_cause() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::global_middleware!(SecurityHeaders::balanced());
    let error = message(runtime(&marked()?)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("nonce"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_public_hash_render_has_one_page_and_no_private_fields()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::global_middleware!(SecurityHeaders::balanced().hash_public_pages());
    Inertia::share("app", "public application")?;
    Inertia::public_shared(&["app"])?;
    let page = runtime(&marked()?)?.render("/public").await?;
    let json: serde_json::Value = serde_json::from_slice(&page.json)
        .map_err(|error| FrameworkError::internal(error.to_string()))?;
    assert_eq!(json["props"]["title"], "Public title");
    assert_eq!(json["props"]["app"], "public application");
    assert!(json["props"].get("auth").is_none());
    assert!(json["props"].get("errors").is_none());
    assert!(json.get("flash").is_none());
    let html = String::from_utf8_lossy(&page.html);
    assert!(!html.contains("private-identity"));
    assert!(!html.contains("nonce="));
    assert!(
        !page
            .headers
            .get("content-security-policy")
            .is_some_and(|csp| csp.contains("nonce-"))
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_group_marker_requires_typed_public_declaration() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new().get("/public", page).into();
    let router = router.public_page_group("/");
    assert_eq!(router.public_page_patterns(), ["/public"]);
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("declaration"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_auth_shared_prop_refuses_without_evaluating_it() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    Inertia::share("auth", serde_json::json!({"id":"private-identity"}))?;
    let error = message(runtime(&marked()?)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("auth"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_opaque_shared_provider_refuses_without_evaluation() -> Result<(), FrameworkError> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Opaque(Arc<AtomicUsize>);
    impl suprnova::ProvidesInertiaProperties for Opaque {
        fn to_inertia_properties(
            &self,
            _: &suprnova::RenderContext<'_>,
        ) -> Result<suprnova::indexmap::IndexMap<String, suprnova::Prop>, FrameworkError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(suprnova::indexmap::IndexMap::new())
        }
    }
    let _container = suprnova::testing::TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    suprnova::App::inertia_registry().share_provider(Opaque(calls.clone()));
    let error = message(runtime(&marked()?)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("provider"),
        "{error}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[suprnova::inertia_root(path = "static_publish/hash.html")]
struct HashRoot;

async fn hash_page(req: Request) -> Response {
    InertiaResponse::new("Public")
        .with_config(
            suprnova::InertiaConfig::new()
                .version("pub-test-v1")
                .development(false)
                .ssr_disabled()
                .root_template(suprnova::InertiaRootTemplate::of::<HashRoot>()),
        )
        .cacheable(PublicProps {
            title: "Public title".to_owned(),
        })
        .map_err(HttpResponse::from)?
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}

async fn rewritten_hash_page(req: Request) -> Response {
    let response = hash_page(req).await?;
    let html = format!(
        "{}<script>injected()</script>",
        String::from_utf8_lossy(response.body())
    );
    Ok(HttpResponse::html(html).with_headers(response.headers()))
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_exact_public_hashes_private_nonce_and_late_injection_refusal()
-> Result<(), FrameworkError> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Production)
            .debug(false)
            .build(),
    );
    let policy = SecurityHeaders::balanced().hash_public_pages();
    suprnova::global_middleware!(policy.clone());
    let router: Router = Router::new()
        .get("/hashed", hash_page)
        .get("/injected", rewritten_hash_page)
        .get("/private", hash_page)
        .into();
    let router = router
        .public_page::<PublicProps>("/hashed", "Public")?
        .public_page::<PublicProps>("/injected", "Public")?;
    let runtime = runtime(&router)?;
    let page = runtime.render("/hashed").await?;
    let csp = page
        .headers
        .get("content-security-policy")
        .ok_or_else(|| FrameworkError::internal("expected public hash CSP"))?;
    let source = |script: &str| {
        format!(
            "'sha256-{}'",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(script.as_bytes()))
        )
    };
    let mut expected = [source("trusted()"), source("module()")];
    expected.sort();
    let scripts = csp
        .split(';')
        .map(str::trim)
        .find(|directive| directive.starts_with("script-src "))
        .unwrap_or_default();
    assert_eq!(scripts, format!("script-src 'self' {}", expected.join(" ")));
    assert_eq!(csp.matches("sha256-").count(), 2);
    assert!(!csp.contains("nonce-"));
    assert!(!String::from_utf8_lossy(&page.html).contains("nonce="));
    let refusal = message(runtime.render("/injected").await)?;
    assert!(
        refusal.contains("/injected") && refusal.contains("rewritten"),
        "{refusal}"
    );
    assert!(!csp.contains(&source("injected()")));
    let client = suprnova::testing::TestClient::new(
        router,
        suprnova::MiddlewareRegistry::new().append(policy),
    );
    let private = client.get("/private").send().await;
    private.assert_ok();
    let csp = private
        .header("content-security-policy")
        .unwrap_or_default();
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .ok_or_else(|| FrameworkError::internal("expected private nonce CSP"))?;
    assert!(private.body_text().contains(&format!("nonce=\"{nonce}\"")));
    assert!(!csp.contains("sha256-"));
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_handler_cannot_swap_public_declaration_for_per_request_props()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new()
        .get("/public", |req: Request| async move {
            InertiaResponse::new("Public")
                .cacheable(PrivateProps {
                    viewer: "private".to_owned(),
                })
                .map_err(HttpResponse::from)?
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let router = router.public_page::<PublicProps>("/public", "Public")?;
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("per-request"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_global_request_hooks_refuse_before_evaluation() -> Result<(), FrameworkError> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Hooks(Arc<AtomicUsize>);
    impl suprnova::InertiaMiddlewareHooks for Hooks {
        fn share(
            &self,
            _: &dyn suprnova::InertiaRequestExt,
        ) -> suprnova::indexmap::IndexMap<String, suprnova::Prop> {
            self.0.fetch_add(1, Ordering::SeqCst);
            suprnova::indexmap::IndexMap::new()
        }
    }
    let _container = suprnova::testing::TestContainer::fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let config = suprnova::InertiaConfig::new().hooks(Hooks(calls.clone()));
    suprnova::global_middleware!(suprnova::InertiaHeadersMiddleware::from_config(&config));
    let error = message(runtime(&marked()?)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("hook"),
        "{error}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_nonce_override_without_global_policy_refuses() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let router: Router = Router::new()
        .get("/public", page)
        .middleware(
            suprnova::SecurityHeadersOverride::new()
                .content_security_policy(Some(suprnova::ContentSecurityPolicy::balanced())),
        )
        .into();
    let router = router.public_page::<PublicProps>("/public", "Public")?;
    let error = message(runtime(&router)?.render("/public").await)?;
    assert!(
        error.contains("/public") && error.contains("nonce"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_003_fix_d_refused_inventory_retains_live_and_records_route()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    Storage::register_memory("refused-inventory");
    let publisher = Publisher::new("refused-inventory", "pub-test-v1", PublishConfig::default())?;
    let first = publisher
        .publish(vec![suprnova::static_publish::PageObject::new(
            "/public",
            bytes::Bytes::from_static(b"previous"),
            bytes::Bytes::from_static(b"{}"),
        )?])
        .await?;
    let router: Router = Router::new().get("/public", page).into();
    let router = router.public_page::<PrivateProps>("/public", "Public")?;
    let runtime = StaticPublication::install(&router, publisher)?;
    let result = runtime.publish().await;
    assert!(
        result.is_err(),
        "a refusal cannot publish a smaller inventory"
    );
    assert!(result.unwrap_err().to_string().contains("/public"));
    let status = runtime.status().await;
    assert_eq!(status.live_version, first.version);
    assert!(status.last_outcome.contains("/public"));
    let durable = Storage::disk("refused-inventory")?
        .read("_pub/status.json")
        .await
        .map_err(|e| FrameworkError::internal(e.to_string()))?
        .to_bytes();
    assert!(String::from_utf8_lossy(&durable).contains("/public"));
    Ok(())
}

#[test]
fn pub_005_fix_p_manual_calls_the_transport_factory() {
    assert!(
        include_str!("../../../manual/frontend-inertia-responses.md")
            .contains("http: staticHttpClient()")
    );
}

#[suprnova::inertia_root(path = "static_publish/foreign-svg.html")]
struct SvgRoot;
#[suprnova::inertia_root(path = "static_publish/foreign-math.html")]
struct MathRoot;

async fn foreign_script_page(req: Request) -> Response {
    let template = if req.path() == "/svg" {
        suprnova::InertiaRootTemplate::of::<SvgRoot>()
    } else {
        suprnova::InertiaRootTemplate::of::<MathRoot>()
    };
    InertiaResponse::new("Public")
        .with_config(
            suprnova::InertiaConfig::new()
                .version("pub-test-v1")
                .development(false)
                .ssr_disabled()
                .root_template(template),
        )
        .cacheable(PublicProps {
            title: "Public".into(),
        })
        .map_err(HttpResponse::from)?
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_fix_n_foreign_content_scripts_refuse_with_route() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::global_middleware!(SecurityHeaders::balanced().hash_public_pages());
    let router: Router = Router::new()
        .get("/svg", foreign_script_page)
        .get("/math", foreign_script_page)
        .into();
    let router = router
        .public_page::<PublicProps>("/svg", "Public")?
        .public_page::<PublicProps>("/math", "Public")?;
    let runtime = runtime(&router)?;
    for path in ["/svg", "/math"] {
        let cause = message(runtime.render(path).await)?;
        assert!(cause.contains(path) && cause.contains("foreign"), "{cause}");
    }
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_fix_o_all_script_directives_remove_broad_inline_sources()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let policy = suprnova::ContentSecurityPolicy::new(
        "default-src 'self'; script-src 'unsafe-inline' 'nonce-{nonce}'; script-src-elem 'self' 'unsafe-inline'; script-src-attr 'unsafe-inline' 'nonce-{nonce}'",
    )?;
    suprnova::global_middleware!(
        SecurityHeaders::balanced()
            .content_security_policy(Some(policy))
            .hash_public_pages()
    );
    let router: Router = Router::new().get("/hashed", hash_page).into();
    let router = router.public_page::<PublicProps>("/hashed", "Public")?;
    let page = runtime(&router)?.render("/hashed").await?;
    let csp = &page.headers["content-security-policy"];
    for directive in csp
        .split(';')
        .map(str::trim)
        .filter(|d| d.starts_with("script-src"))
    {
        assert!(
            !directive.contains("unsafe-inline") && !directive.contains("nonce-"),
            "{directive}"
        );
    }
    assert!(csp.contains("sha256-"));
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn pub_001_fix_f_hsts_is_recorded_and_applied_at_secure_delivery()
-> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Production)
            .debug(false)
            .build(),
    );
    let policy = SecurityHeaders::balanced().hash_public_pages();
    suprnova::global_middleware!(policy.clone());
    let router: Router = Router::new().get("/secure", hash_page).into();
    let router = router.public_page::<PublicProps>("/secure", "Public")?;
    let runtime = runtime(&router)?;
    Storage::register_memory("hsts-assets");
    let source = Storage::disk("hsts-assets")?;
    for key in ["assets/main.css", "assets/main.js"] {
        source
            .write(key, "asset")
            .await
            .map_err(|e| FrameworkError::internal(e.to_string()))?;
    }
    runtime.assets(source)?;
    let client = suprnova::testing::TestClient::new(
        router,
        suprnova::MiddlewareRegistry::new().append(policy),
    );
    let secure = client.get("https://localhost/secure").send().await;
    secure.assert_ok();
    assert!(
        secure.header("strict-transport-security").is_some(),
        "public HTTPS preserves HSTS"
    );
    let stored = runtime.render("/secure").await?;
    assert!(
        stored.headers.contains_key("strict-transport-security"),
        "the CDN needs the configured value"
    );
    let insecure = client
        .get("http://localhost/secure")
        .header("X-Forwarded-Proto", "https")
        .send()
        .await;
    insecure.assert_ok();
    assert!(
        insecure.header("strict-transport-security").is_none(),
        "untrusted HTTP cannot enable HSTS"
    );
    Ok(())
}
