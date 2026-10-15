//! Presets fail closed at the HTTP boundary and share one request nonce.

use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    AppConfig, Config, CspNonce, Environment, HttpResponse, MiddlewareRegistry, Router,
    SecurityHeaders,
};

pub(super) fn production() -> suprnova::testing::TestContainerGuard {
    let guard = TestContainer::fake();
    Config::register(
        AppConfig::builder()
            .environment(Environment::Production)
            .debug(false)
            .build(),
    );
    guard
}

pub(super) fn client(policy: SecurityHeaders) -> TestClient {
    TestClient::new(
        Router::new().get("/", |req: suprnova::Request| async move {
            assert_eq!(req.csp_nonce(), CspNonce::current());
            Ok(HttpResponse::html(req.csp_nonce().unwrap_or_default()))
        }),
        MiddlewareRegistry::new().append(policy),
    )
}

#[tokio::test]
async fn sec_001_strict_prevents_framing_and_unnonced_scripts_and_sets_secure_hsts() {
    let _scope = production();
    let response = client(SecurityHeaders::strict())
        .get("https://localhost/")
        .send()
        .await;
    response
        .assert_ok()
        .assert_header("x-frame-options", "DENY")
        .assert_header(
            "strict-transport-security",
            "max-age=63072000; includeSubDomains",
        );
    let csp = response.header("content-security-policy").expect("CSP");
    assert_eq!(
        csp,
        format!(
            "default-src 'self'; script-src 'nonce-{}' 'strict-dynamic'; style-src 'self' 'nonce-{}'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
            response.body_text(),
            response.body_text()
        )
    );
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains(&format!(
        "script-src 'nonce-{}' 'strict-dynamic'",
        response.body_text()
    )));
    assert!(!csp.contains("'unsafe-inline'"));
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-embedder-policy", "require-corp"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        response.assert_header(name, value);
    }
    let permissions = response.header("permissions-policy").expect("permissions");
    for feature in ["camera", "microphone", "geolocation", "payment", "usb"] {
        assert!(permissions.contains(&format!("{feature}=()")));
    }
}

#[tokio::test]
async fn sec_002_balanced_and_relaxed_have_the_documented_headers() {
    let _scope = production();
    let response = client(SecurityHeaders::balanced())
        .get("https://localhost/")
        .send()
        .await;
    response
        .assert_header("strict-transport-security", "max-age=31536000")
        .assert_header("x-frame-options", "SAMEORIGIN")
        .assert_header("cross-origin-opener-policy", "same-origin-allow-popups")
        .assert_header("cross-origin-resource-policy", "same-site");
    assert!(response.header("cross-origin-embedder-policy").is_none());
    let permissions = response.header("permissions-policy").expect("permissions");
    for feature in ["camera", "microphone", "geolocation", "usb"] {
        assert!(
            permissions.contains(&format!("{feature}=()")),
            "{permissions}"
        );
    }
    assert!(
        !permissions.contains("payment"),
        "balanced keeps payment for wallet buttons: {permissions}"
    );
    assert!(
        response
            .header("content-security-policy")
            .unwrap()
            .contains("style-src 'self' 'unsafe-inline'")
    );
    // Same-origin script files run (the Live components' tags carry no
    // nonce), so balanced must not carry 'strict-dynamic', which switches
    // 'self' off in browsers that support it.
    let csp = response.header("content-security-policy").unwrap();
    assert!(csp.contains("script-src 'self' 'nonce-"), "{csp}");
    assert!(!csp.contains("'strict-dynamic'"), "{csp}");
    let relaxed = client(SecurityHeaders::relaxed())
        .get("https://localhost/")
        .send()
        .await;
    for name in [
        "content-security-policy",
        "permissions-policy",
        "cross-origin-opener-policy",
        "cross-origin-embedder-policy",
        "cross-origin-resource-policy",
    ] {
        assert!(relaxed.header(name).is_none(), "{name}");
    }
    relaxed
        .assert_header("x-frame-options", "SAMEORIGIN")
        .assert_header("x-content-type-options", "nosniff")
        .assert_header("referrer-policy", "strict-origin-when-cross-origin")
        .assert_header("strict-transport-security", "max-age=31536000");
    assert!(relaxed.body_text().is_empty());
}

#[tokio::test]
async fn sec_002_hsts_obeys_https_local_and_preload_rules() {
    let _scope = production();
    let http = client(SecurityHeaders::strict())
        .get("/")
        .header("x-forwarded-proto", "https")
        .send()
        .await;
    assert!(http.header("strict-transport-security").is_none());
    let preload = client(SecurityHeaders::strict().hsts_preload(true))
        .get("https://localhost/")
        .send()
        .await;
    assert!(
        preload
            .header("strict-transport-security")
            .unwrap()
            .ends_with("; preload")
    );
    for environment in [Environment::Local, Environment::Development] {
        Config::register(AppConfig::builder().environment(environment).build());
        assert!(
            client(SecurityHeaders::strict())
                .get("https://localhost/")
                .send()
                .await
                .header("strict-transport-security")
                .is_none()
        );
    }
}

#[tokio::test]
async fn sec_002_report_only_reports_without_an_enforced_nonce_policy() {
    let _scope = production();
    let response = client(
        SecurityHeaders::balanced()
            .report_only(true)
            .report_to(Some("https://reports.example/csp"))
            .unwrap(),
    )
    .get("/")
    .send()
    .await;
    assert!(response.header("content-security-policy").is_none());
    assert!(
        response
            .header("content-security-policy-report-only")
            .unwrap()
            .contains("report-to csp")
    );
    response.assert_header("reporting-endpoints", "csp=\"https://reports.example/csp\"");
}

#[tokio::test]
async fn sec_002_json_and_non_html_never_carry_a_nonce_policy() {
    let _scope = production();
    for json in [true, false] {
        let client = TestClient::new(
            Router::new().get("/", move |_| async move {
                Ok(if json {
                    HttpResponse::json(serde_json::json!({"ok":true}))
                } else {
                    HttpResponse::text("plain")
                })
            }),
            MiddlewareRegistry::new().append(SecurityHeaders::strict()),
        );
        let response = client.get("/").send().await;
        assert!(response.header("content-security-policy").is_none());
        response.assert_header("x-content-type-options", "nosniff");
    }
}

#[tokio::test]
async fn sec_003_nonces_are_fresh_scoped_128_bit_base64url_and_propagate() {
    let _scope = production();
    let client = client(SecurityHeaders::balanced());
    let first = client.get("/").send().await.body_text();
    let second = client.get("/").send().await.body_text();
    assert_ne!(first, second);
    assert_eq!(first.len(), 22);
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    );
    assert!(CspNonce::current().is_none());
    let client = TestClient::new(
        Router::new().get("/", |req: suprnova::Request| async move {
            let expected = req.csp_nonce();
            let actual = tokio::spawn(CspNonce::propagate(async { CspNonce::current() }))
                .await
                .unwrap();
            assert_eq!(expected, actual);
            Ok(HttpResponse::html(actual.unwrap()))
        }),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    client.get("/").send().await.assert_ok();
}

#[tokio::test]
async fn sec_002_local_policy_uses_the_installed_vite_origin_and_hmr_websocket() {
    let _scope = production();
    Config::register(AppConfig::builder().environment(Environment::Local).build());
    suprnova::Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .register_globally(false)
            .vite_dev_server("https://vite.example:6123"),
    )
    .expect("development config");
    let response = client(SecurityHeaders::balanced()).get("/").send().await;
    let csp = response.header("content-security-policy").unwrap();
    assert!(csp.contains("https://vite.example:6123"), "{csp}");
    assert!(
        csp.contains("connect-src 'self' https://vite.example:6123 wss://vite.example:6123"),
        "{csp}"
    );
}

#[test]
fn sec_001_from_env_rejects_invalid_values_and_selects_every_preset() {
    const CHILD: &str = "SUPRNOVA_SECURITY_TEST_CHILD";
    for (variable, value, valid) in [
        ("SECURITY_HEADERS", "strict", true),
        ("SECURITY_HEADERS", "balanced", true),
        ("SECURITY_HEADERS", "relaxed", true),
        ("SECURITY_HEADERS", "off", true),
        ("SECURITY_HEADERS", "typo", false),
        ("SECURITY_HEADERS_CSP_REPORT_ONLY", "perhaps", false),
        ("SECURITY_HEADERS_HSTS_PRELOAD", "perhaps", false),
        ("SECURITY_HEADERS_CSP_REPORT_TO", "not a URL", false),
        ("SECURITY_HEADERS_CSP_REPORT_ONLY", "true", true),
        ("SECURITY_HEADERS_HSTS_PRELOAD", "true", true),
        (
            "SECURITY_HEADERS_CSP_REPORT_TO",
            "https://reports.example/csp",
            true,
        ),
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "policies::sec_001_from_env_child", "--nocapture"])
            .env_remove("SECURITY_HEADERS")
            .env_remove("SECURITY_HEADERS_CSP_REPORT_ONLY")
            .env_remove("SECURITY_HEADERS_CSP_REPORT_TO")
            .env_remove("SECURITY_HEADERS_HSTS_PRELOAD")
            .env(CHILD, if valid { "valid" } else { variable })
            .env(variable, value)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{variable}={value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn sec_001_from_env_child() {
    let Ok(expected) = std::env::var("SUPRNOVA_SECURITY_TEST_CHILD") else {
        return;
    };
    let policy = SecurityHeaders::from_env();
    if expected == "valid" {
        assert!(policy.is_ok(), "{policy:?}");
    } else {
        assert!(policy.unwrap_err().to_string().contains(&expected));
    }
}

struct EmptyHtmlError;

#[suprnova::async_trait]
impl suprnova::Middleware for EmptyHtmlError {
    async fn handle(
        &self,
        _request: suprnova::Request,
        _next: suprnova::Next,
    ) -> suprnova::Response {
        Err(HttpResponse::html("").status(403))
    }
}

#[tokio::test]
async fn sec_002_server_error_callback_json_has_no_nonce_policy() {
    let _scope = production();
    suprnova::Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .register_globally(false),
    )
    .unwrap();
    suprnova::Inertia::handle_exceptions_using(|error| {
        Some(error.render("Error", serde_json::json!({})))
    });
    let client = TestClient::new(
        Router::new(),
        MiddlewareRegistry::new()
            .append(EmptyHtmlError)
            .append(SecurityHeaders::strict()),
    );
    let response = client.get("/missing").inertia().send().await;
    response
        .assert_status(403)
        .assert_header("x-content-type-options", "nosniff");
    assert!(
        response
            .header("content-type")
            .unwrap()
            .starts_with("application/json")
    );
    assert!(response.header("content-security-policy").is_none());
    assert_eq!(response.json()["component"], "Error");
}

#[tokio::test]
async fn sec_002_report_only_builder_preserves_typed_policy_updates_and_switches_back() {
    let _scope = production();
    let policy = SecurityHeaders::strict()
        .report_only(true)
        .content_security_policy(Some(suprnova::ContentSecurityPolicy::balanced()))
        .report_only(true);
    let report = client(policy.clone()).get("/").send().await;
    assert!(report.header("content-security-policy").is_none());
    assert!(
        report
            .header("content-security-policy-report-only")
            .unwrap()
            .contains(&report.body_text())
    );
    let enforced = client(policy.report_only(false)).get("/").send().await;
    assert!(
        enforced
            .header("content-security-policy-report-only")
            .is_none()
    );
    assert!(
        enforced
            .header("content-security-policy")
            .unwrap()
            .contains(&enforced.body_text())
    );
}
