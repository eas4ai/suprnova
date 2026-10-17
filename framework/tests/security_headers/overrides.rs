//! Route policies compose with groups and do not replace handler-owned headers.

use suprnova::testing::TestClient;
use suprnova::{
    ContentSecurityPolicy, HttpResponse, Middleware, MiddlewareRegistry, Next, Request, Response,
    Router, SecurityHeaders, SecurityHeadersOverride,
};

#[tokio::test]
async fn sec_004_route_and_group_can_change_or_disable_every_header() {
    let _scope = super::policies::production();
    let names = [
        "content-security-policy",
        "content-security-policy-report-only",
        "strict-transport-security",
        "x-frame-options",
        "x-content-type-options",
        "referrer-policy",
        "permissions-policy",
        "cross-origin-opener-policy",
        "cross-origin-embedder-policy",
        "cross-origin-resource-policy",
    ];
    for name in names {
        let override_policy = SecurityHeadersOverride::new().header(name, None).unwrap();
        let router = Router::new()
            .group("/group", |routes| {
                routes.get("/", |_| async { Ok(HttpResponse::html("group")) })
            })
            .middleware(override_policy.clone())
            .try_finalize()
            .unwrap()
            .get("/route", |_| async { Ok(HttpResponse::html("route")) })
            .middleware(override_policy);
        let client = TestClient::new(
            router,
            MiddlewareRegistry::new().append(
                SecurityHeaders::strict()
                    .header(
                        "content-security-policy-report-only",
                        Some("script-src 'nonce-{nonce}'"),
                    )
                    .unwrap(),
            ),
        );
        for path in ["https://localhost/group", "https://localhost/route"] {
            let response = client.get(path).send().await;
            response.assert_ok();
            assert!(response.header(name).is_none(), "{name} on {path}");
        }
    }
    let router = Router::new()
        .group("/group", |routes| {
            routes.get("/", |_| async { Ok(HttpResponse::html("group")) })
        })
        .middleware(
            SecurityHeadersOverride::new()
                .header("x-frame-options", Some("DENY"))
                .unwrap(),
        )
        .try_finalize()
        .unwrap()
        .get("/route", |_| async { Ok(HttpResponse::html("route")) })
        .middleware(
            SecurityHeadersOverride::new()
                .header("referrer-policy", Some("no-referrer"))
                .unwrap(),
        );
    let client = TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::balanced()),
    );
    client
        .get("/group")
        .send()
        .await
        .assert_header("x-frame-options", "DENY");
    client
        .get("/route")
        .send()
        .await
        .assert_header("referrer-policy", "no-referrer");
}

#[tokio::test]
async fn sec_004_handler_headers_win_including_live_transport_policies() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new()
            .get("/", |_| async {
                Ok(HttpResponse::html("handler")
                    .header("Content-Security-Policy", "default-src 'none'")
                    .header("X-Frame-Options", "SAMEORIGIN")
                    .header("Referrer-Policy", "same-origin"))
            })
            .middleware(
                SecurityHeadersOverride::new()
                    .header("x-frame-options", None)
                    .unwrap(),
            ),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    client
        .get("/")
        .send()
        .await
        .assert_header("content-security-policy", "default-src 'none'")
        .assert_header("x-frame-options", "SAMEORIGIN")
        .assert_header("referrer-policy", "same-origin");
}

#[tokio::test]
async fn sec_004_route_can_introduce_or_disable_a_nonce_before_rendering() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new()
            .get("/", |request: Request| async move {
                Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
            })
            .middleware(
                SecurityHeadersOverride::new()
                    .content_security_policy(Some(ContentSecurityPolicy::strict())),
            ),
        MiddlewareRegistry::new().append(SecurityHeaders::relaxed()),
    );
    let response = client.get("/").send().await;
    assert_eq!(response.body_text().len(), 22);
    assert!(
        response
            .header("content-security-policy")
            .unwrap()
            .contains(&response.body_text())
    );
    let client = TestClient::new(
        Router::new()
            .get("/", |request: Request| async move {
                Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
            })
            .middleware(SecurityHeadersOverride::new().content_security_policy(None)),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    let response = client.get("/").send().await;
    assert!(response.body_text().is_empty());
    assert!(response.header("content-security-policy").is_none());
}

struct ShortCircuit;
#[suprnova::async_trait]
impl Middleware for ShortCircuit {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(HttpResponse::html("error").status(403))
    }
}

#[tokio::test]
async fn sec_001_security_runs_outside_short_circuiting_global_error_middleware() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new(),
        MiddlewareRegistry::new()
            .append(ShortCircuit)
            .append(SecurityHeaders::strict()),
    );
    client
        .get("/missing")
        .send()
        .await
        .assert_status(403)
        .assert_header("x-frame-options", "DENY")
        .assert_header("x-content-type-options", "nosniff");
}

#[tokio::test]
async fn sec_001_server_replaced_5xx_keeps_headers_but_uses_its_own_no_script_csp() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new().get("/", |_| async {
            Err(HttpResponse::html("private error").status(500))
        }),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    let response = client.get("/").header("Accept", "text/html").send().await;
    response
        .assert_status(500)
        .assert_header("x-frame-options", "DENY")
        .assert_header("x-content-type-options", "nosniff");
    assert!(!response.body_text().contains("private error"));
    let csp = response.header("content-security-policy").unwrap();
    assert!(
        csp.contains("script-src 'none'")
            || (csp.contains("default-src 'none'") && !csp.contains("script-src")),
        "{csp}"
    );
}

#[tokio::test]
async fn sec_001_a_caught_panic_keeps_security_headers_on_the_server_error_page() {
    let _scope = super::policies::production();
    let client = TestClient::new(
        Router::new().get("/", |_| async { panic!("security panic private detail") }),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    let response = client
        .get("https://localhost/")
        .header("Accept", "text/html")
        .send()
        .await;
    response
        .assert_status(500)
        .assert_header("x-frame-options", "DENY")
        .assert_header("x-content-type-options", "nosniff")
        .assert_header(
            "strict-transport-security",
            "max-age=63072000; includeSubDomains",
        );
    assert!(
        !response
            .body_text()
            .contains("security panic private detail")
    );
    assert!(
        response
            .header("content-security-policy")
            .unwrap()
            .contains("default-src 'none'")
    );
}

#[tokio::test]
async fn sec_004_route_and_group_can_disable_generated_reporting_endpoints() {
    let _scope = super::policies::production();
    let policy = SecurityHeaders::strict()
        .report_to(Some("https://reports.example/csp"))
        .unwrap();
    let router = Router::new()
        .group("/group", |routes| {
            routes.get("/", |_| async { Ok(HttpResponse::html("group")) })
        })
        .middleware(
            SecurityHeadersOverride::new()
                .header("reporting-endpoints", None)
                .unwrap(),
        )
        .try_finalize()
        .unwrap()
        .get("/route", |_| async { Ok(HttpResponse::html("route")) })
        .middleware(
            SecurityHeadersOverride::new()
                .header("reporting-endpoints", None)
                .unwrap(),
        );
    let client = TestClient::new(router, MiddlewareRegistry::new().append(policy));
    for path in ["/group", "/route"] {
        let response = client.get(path).send().await;
        response.assert_ok();
        assert!(response.header("reporting-endpoints").is_none());
        assert!(
            !response
                .header("content-security-policy")
                .unwrap()
                .contains("report-to")
        );
    }
}

#[tokio::test]
async fn sec_004_removing_report_only_keeps_the_enforcing_policy_and_nonce() {
    let _scope = super::policies::production();
    let router = Router::new()
        .get("/", |request: Request| async move {
            Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
        })
        .middleware(
            SecurityHeadersOverride::new()
                .header("Content-Security-Policy-Report-Only", None)
                .unwrap(),
        );
    let response = TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    )
    .get("/")
    .send()
    .await;
    assert_eq!(
        response.body_text().len(),
        22,
        "enforcing CSP still requires a nonce"
    );
    assert!(
        response
            .header("content-security-policy")
            .expect("enforcing policy")
            .contains(&response.body_text())
    );
    assert!(
        response
            .header("content-security-policy-report-only")
            .is_none()
    );
}

#[tokio::test]
async fn sec_004_enforcing_and_report_only_policies_change_independently() {
    let _scope = super::policies::production();
    for via_route in [false, true] {
        for removed in [
            None,
            Some("content-security-policy"),
            Some("content-security-policy-report-only"),
        ] {
            let mut router: Router = Router::new()
                .get("/", |request: Request| async move {
                    Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
                })
                .into();
            if let Some(name) = removed.filter(|_| via_route) {
                router = router
                    .get("/override", |request: Request| async move {
                        Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
                    })
                    .middleware(SecurityHeadersOverride::new().header(name, None).unwrap())
                    .into();
            }
            let mut policy = SecurityHeaders::strict()
                .header(
                    "Content-Security-Policy-Report-Only",
                    Some("script-src 'nonce-{nonce}'; object-src 'none'"),
                )
                .unwrap();
            if let Some(name) = removed.filter(|_| !via_route) {
                policy = policy.header(name, None).unwrap();
            }
            let response = TestClient::new(router, MiddlewareRegistry::new().append(policy))
                .get(if removed.is_some() && via_route {
                    "/override"
                } else {
                    "/"
                })
                .send()
                .await;
            let nonce = response.body_text();
            assert_eq!(nonce.len(), 22);
            for name in [
                "content-security-policy",
                "content-security-policy-report-only",
            ] {
                if removed == Some(name) {
                    assert!(response.header(name).is_none());
                } else {
                    assert!(
                        response
                            .header(name)
                            .unwrap_or_else(|| panic!("missing {name}"))
                            .contains(&format!("'nonce-{nonce}'"))
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn sec_004_route_can_add_a_report_only_nonce_policy_on_its_own() {
    let _scope = super::policies::production();
    let router = Router::new()
        .get("/", |request: Request| async move {
            Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
        })
        .middleware(
            SecurityHeadersOverride::new()
                .header(
                    "content-security-policy-report-only",
                    Some("script-src 'nonce-{nonce}'"),
                )
                .unwrap(),
        );
    let response = TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::relaxed()),
    )
    .get("/")
    .send()
    .await;
    assert!(response.header("content-security-policy").is_none());
    assert_eq!(response.body_text().len(), 22);
    assert!(
        response
            .header("content-security-policy-report-only")
            .unwrap()
            .contains(&response.body_text())
    );
}
