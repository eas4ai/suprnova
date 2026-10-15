//! Nonce policies render Complete pages afresh while stitched documents
//! retain shared shells and fill every nonce hole from the current request.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Method;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::render_cache::{
    CoordinatorConfig, FreshnessPolicy, HintsConfig, L1Config, RenderCache, RenderCacheConfig,
    RenderCachePolicy, RepresentationClass,
};
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::{
    App, ContentSecurityPolicy, Crypt, EncryptionKey, HttpResponse, MiddlewareRegistry, Request,
    Response, Router, SecurityHeaders, SecurityHeadersOverride, StatusCode, handle_request,
};

static COMPLETE_RENDERS: AtomicUsize = AtomicUsize::new(0);

struct CacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for CacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

async fn complete_document(request: Request) -> Response {
    let render = COMPLETE_RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
    let nonce = request.csp_nonce().unwrap_or_default();
    Ok(HttpResponse::html(format!(
        "<html><script nonce=\"{nonce}\">window.render={render}</script></html>"
    )))
}

async fn dispatch(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
) -> (hyper::HeaderMap, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .expect("serve test request");
    });
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect request");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move { connection.await.expect("HTTP connection") });
    let request = hyper::Request::builder()
        .uri("/security-complete")
        .header("host", "127.0.0.1")
        .header("accept", "text/html")
        .body(Full::new(Bytes::new()))
        .expect("request");
    let response = sender.send_request(request).await.expect("send request");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (headers, String::from_utf8(body.to_vec()).expect("HTML"))
}

async fn complete_fixture(
    override_policy: Option<SecurityHeadersOverride>,
) -> (Arc<Router>, suprnova::testing::TestDatabase) {
    App::init();
    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    suprnova::middleware::clear_global_middleware_for_test();
    COMPLETE_RENDERS.store(0, Ordering::SeqCst);
    let database = TestDatabase::fresh::<CacheMigrator>()
        .await
        .expect("test database");
    let route = Router::new().get("/security-complete", complete_document);
    let router: Router = match override_policy {
        Some(policy) => route.middleware(policy).into(),
        None => route.into(),
    };
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("public cache policy");
    let router = router
        .try_render_cache("/security-complete", policy)
        .expect("cache route");
    let mut config = RenderCacheConfig::from_env().expect("render cache config");
    config.enabled = true;
    config.l1 = L1Config::Disabled;
    config.hints = HintsConfig::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, config)
        .await
        .expect("install render cache");
    (Arc::new(router), database)
}

fn nonce_from_policy(headers: &hyper::HeaderMap) -> &str {
    headers["content-security-policy"]
        .to_str()
        .expect("CSP text")
        .split("'nonce-")
        .nth(1)
        .and_then(|value| value.split('\'').next())
        .expect("nonce source")
}

#[tokio::test]
#[serial_test::serial]
async fn sec_005_complete_nonce_pages_render_twice_and_are_never_stored() {
    let _container = TestContainer::fake();
    let (router, _database) = complete_fixture(None).await;
    suprnova::middleware::register_global_middleware(SecurityHeaders::balanced());
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    let (first_headers, first_body) = dispatch(Arc::clone(&router), Arc::clone(&middleware)).await;
    let (second_headers, second_body) = dispatch(router, middleware).await;
    assert_eq!(COMPLETE_RENDERS.load(Ordering::SeqCst), 2);
    assert_ne!(
        nonce_from_policy(&first_headers),
        nonce_from_policy(&second_headers)
    );
    assert!(first_body.contains(&format!("nonce=\"{}\"", nonce_from_policy(&first_headers))));
    assert!(second_body.contains(&format!("nonce=\"{}\"", nonce_from_policy(&second_headers))));
    assert!(
        RenderCache::inspect_route_for_test("/security-complete")
            .await
            .is_none()
    );
}

#[tokio::test]
#[serial_test::serial]
async fn sec_005_enabling_nonce_policy_bypasses_an_existing_complete_rendering() {
    let _container = TestContainer::fake();
    let (router, _database) = complete_fixture(None).await;
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    dispatch(Arc::clone(&router), Arc::clone(&middleware)).await;
    dispatch(Arc::clone(&router), middleware).await;
    assert_eq!(
        COMPLETE_RENDERS.load(Ordering::SeqCst),
        1,
        "precondition: Complete hit"
    );
    assert!(
        RenderCache::inspect_route_for_test("/security-complete")
            .await
            .is_some()
    );
    suprnova::middleware::register_global_middleware(SecurityHeaders::balanced());
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    let (headers, body) = dispatch(router, middleware).await;
    assert_eq!(COMPLETE_RENDERS.load(Ordering::SeqCst), 2);
    assert!(body.contains(&format!("nonce=\"{}\"", nonce_from_policy(&headers))));
}

#[tokio::test]
#[serial_test::serial]
async fn sec_001_a_complete_cache_hit_keeps_outer_security_headers() {
    let _container = TestContainer::fake();
    let (router, _database) = complete_fixture(None).await;
    // Install after RenderCache deliberately: SecurityHeaders must still
    // wrap the cache's short circuit when the application registered it late.
    suprnova::middleware::register_global_middleware(SecurityHeaders::relaxed());
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    dispatch(Arc::clone(&router), Arc::clone(&middleware)).await;
    let (headers, _) = dispatch(router, middleware).await;
    assert_eq!(
        COMPLETE_RENDERS.load(Ordering::SeqCst),
        1,
        "the second request hit storage"
    );
    assert_eq!(headers["x-frame-options"], "SAMEORIGIN");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(
        headers["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn sec_004_route_header_overrides_apply_to_a_complete_cache_hit() {
    let _container = TestContainer::fake();
    let override_policy = SecurityHeadersOverride::new()
        .header("referrer-policy", Some("no-referrer"))
        .expect("referrer override")
        .header("x-frame-options", None)
        .expect("disable frame header");
    let (router, _database) = complete_fixture(Some(override_policy)).await;
    suprnova::middleware::register_global_middleware(SecurityHeaders::relaxed());
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    dispatch(Arc::clone(&router), Arc::clone(&middleware)).await;
    let (headers, _) = dispatch(router, middleware).await;
    assert_eq!(
        COMPLETE_RENDERS.load(Ordering::SeqCst),
        1,
        "the second request hit storage"
    );
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert!(!headers.contains_key("x-frame-options"));
    assert_eq!(headers["x-content-type-options"], "nosniff");
}

#[tokio::test]
#[serial_test::serial]
async fn sec_004_sec_005_route_nonce_override_precedes_a_warmed_complete_cache() {
    let _container = TestContainer::fake();
    let override_policy = SecurityHeadersOverride::new()
        .content_security_policy(Some(ContentSecurityPolicy::strict()));
    let (router, _database) = complete_fixture(Some(override_policy)).await;
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    dispatch(Arc::clone(&router), Arc::clone(&middleware)).await;
    dispatch(Arc::clone(&router), middleware).await;
    assert_eq!(
        COMPLETE_RENDERS.load(Ordering::SeqCst),
        1,
        "precondition: Complete hit"
    );
    suprnova::middleware::register_global_middleware(SecurityHeaders::relaxed());
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    let (headers, body) = dispatch(router, middleware).await;
    assert_eq!(COMPLETE_RENDERS.load(Ordering::SeqCst), 2);
    assert!(body.contains(&format!("nonce=\"{}\"", nonce_from_policy(&headers))));
}

#[tokio::test]
#[serial_test::serial]
async fn sec_003_sec_005_stitched_hits_fill_body_and_header_with_the_request_nonce() {
    use crate::render_cache_stitch_support::{
        STITCHED_PATH, attribute, boot_with_security_headers, decoded_snapshot, dispatch,
        handler_renders, island_tag,
    };
    let harness = boot_with_security_headers().await;
    let first = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .expect("published shell")
            .kind,
        suprnova_live::render_cache::entry::EntryKind::Composite
    );
    let renders = handler_renders(STITCHED_PATH);
    let first_hit = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    let second_hit = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-c")],
    )
    .await;
    assert_eq!(
        handler_renders(STITCHED_PATH),
        renders,
        "both requests hit the shell"
    );
    for response in [&first, &first_hit, &second_hit] {
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let nonce = response
            .header("x-test-request-nonce")
            .expect("request nonce");
        assert!(
            response
                .header("content-security-policy")
                .expect("CSP")
                .contains(&format!("'nonce-{nonce}'"))
        );
        let html = response.text();
        let script = html
            .split('<')
            .find(|tag| tag.starts_with("script") && !tag.contains("type=\"application/json\""))
            .and_then(|tag| tag.split('>').next())
            .expect("executable script opening tag");
        assert_eq!(attribute(script, "nonce"), nonce);
        let snapshot = decoded_snapshot(island_tag(&html, "stitch-counter"));
        assert!(
            snapshot["body"]["scope"]
                .as_str()
                .is_some_and(|scope| !scope.is_empty())
        );
    }
    assert_ne!(
        first_hit.header("x-test-request-nonce"),
        second_hit.header("x-test-request-nonce")
    );
    assert_eq!(first_hit.header("x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(first_hit.header("x-content-type-options"), Some("nosniff"));
}
