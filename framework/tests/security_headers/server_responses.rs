//! Built-in endpoints and upgrade failures obey the application's registered policy.

use suprnova::testing::TestClient;
use suprnova::{Middleware, MiddlewareRegistry, Next, Request, Response, Router, SecurityHeaders};

fn client(router: Router) -> TestClient {
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    )
}

fn assert_strict(response: &suprnova::testing::TestResponse, secure: bool) {
    response
        .assert_header("x-content-type-options", "nosniff")
        .assert_header("x-frame-options", "DENY");
    assert_eq!(
        response.header("strict-transport-security"),
        secure.then_some("max-age=63072000; includeSubDomains")
    );
    assert!(response.header("content-security-policy").is_none());
}

#[tokio::test]
async fn sec_001_health_responses_use_the_registered_policy() {
    let _scope = super::policies::production();
    let client = client(Router::new());
    for scheme in ["http", "https"] {
        let response = client
            .get(format!("{scheme}://localhost/_suprnova/health"))
            .send()
            .await;
        response.assert_ok();
        assert_strict(&response, scheme == "https");
    }
}

#[cfg(feature = "localization")]
#[tokio::test]
async fn sec_001_catalog_responses_use_the_registered_policy() {
    let _scope = super::policies::production();
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("en")).unwrap();
    std::fs::write(directory.path().join("en/app.ftl"), "greeting = Hello\n").unwrap();
    let translator = suprnova::FluentTranslator::from_dir(
        directory.path(),
        &suprnova::LocalizationConfig {
            default_locale: suprnova::Locale::parse("en").unwrap(),
            fallback_locale: suprnova::Locale::parse("en").unwrap(),
            use_isolating: false,
            detection: vec![],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents: Default::default(),
        },
    )
    .unwrap();
    suprnova::App::bind::<dyn suprnova::Translator>(std::sync::Arc::new(translator));
    let client = client(Router::new());
    for scheme in ["http", "https"] {
        let response = client
            .get(format!("{scheme}://localhost/_suprnova/lang/en.ftl"))
            .send()
            .await;
        response.assert_ok();
        assert_strict(&response, scheme == "https");
        let etag = response.header("etag").unwrap();
        let cached = client
            .get(format!("{scheme}://localhost/_suprnova/lang/en.ftl"))
            .header("if-none-match", etag)
            .send()
            .await;
        cached.assert_status(304);
        assert_strict(&cached, scheme == "https");
    }
}

struct Panicking;
#[suprnova::async_trait]
impl Middleware for Panicking {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        panic!("upgrade middleware regression")
    }
}

struct Socket;
#[suprnova::async_trait]
impl suprnova::ws::WebSocketHandler for Socket {
    async fn handle(
        &self,
        _socket: suprnova::ws::WsSocket,
        _request: Request,
    ) -> Result<(), suprnova::FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
async fn sec_001_websocket_panic_responses_use_the_registered_policy() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new().ws("/socket", Socket),
        MiddlewareRegistry::new()
            .append(SecurityHeaders::strict())
            .append(Panicking),
    );
    let response = client
        .get("https://localhost/socket")
        .header("origin", "https://localhost")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await;
    response.assert_status(500);
    assert_strict(&response, true);
}

#[tokio::test]
async fn sec_001_builtin_policy_is_scoped_to_each_registry_and_local_environment() {
    let _scope = super::policies::production();
    let strict = client(Router::new());
    let unconfigured = TestClient::new(Router::new(), MiddlewareRegistry::new());
    let relaxed = TestClient::new(
        Router::new(),
        MiddlewareRegistry::new().append(SecurityHeaders::relaxed()),
    );
    strict
        .get("/_suprnova/health")
        .send()
        .await
        .assert_header("x-frame-options", "DENY");
    assert!(
        unconfigured
            .get("/_suprnova/health")
            .send()
            .await
            .header("x-frame-options")
            .is_none()
    );
    relaxed
        .get("/_suprnova/health")
        .send()
        .await
        .assert_header("x-frame-options", "SAMEORIGIN");
    suprnova::Config::register(
        suprnova::AppConfig::builder()
            .environment(suprnova::Environment::Local)
            .debug(false)
            .build(),
    );
    assert_strict(
        &strict
            .get("https://localhost/_suprnova/health")
            .send()
            .await,
        false,
    );
}

#[tokio::test]
async fn sec_001_websocket_origin_rejections_use_the_registered_policy() {
    let _scope = super::policies::production();
    let response = client(Router::new().ws("/socket", Socket))
        .get("https://localhost/socket")
        .header("origin", "https://evil.example")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await;
    response.assert_status(403);
    assert_strict(&response, true);
}

#[tokio::test]
async fn sec_004_websocket_panic_preserves_the_effective_route_policy() {
    let _scope = super::policies::production();
    let router = suprnova::ws!("/socket", Socket)
        .middleware(
            suprnova::SecurityHeadersOverride::new()
                .header("x-frame-options", None)
                .unwrap()
                .header("referrer-policy", Some("same-origin"))
                .unwrap(),
        )
        .register(Router::new());
    let client = TestClient::new(
        router,
        MiddlewareRegistry::new()
            .append(SecurityHeaders::strict())
            .append(Panicking),
    );
    let response = client
        .get("https://localhost/socket")
        .header("origin", "https://localhost")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await;
    response
        .assert_status(500)
        .assert_header("x-content-type-options", "nosniff")
        .assert_header("referrer-policy", "same-origin");
    assert!(response.header("x-frame-options").is_none());
}
