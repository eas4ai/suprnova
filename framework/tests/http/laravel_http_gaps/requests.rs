//! The request gaps (PAR-118): `Request::ajax()` compares
//! `X-Requested-With` exactly, `Request::host()` lowercases, strips only a
//! numeric port and refuses a host with a byte no host has, `TrustHosts`
//! answers `400 Bad request.` to a host the application does not serve,
//! and `Cookie::forget` takes its path, domain and SameSite from the
//! session configuration.
//!
//! `TrustHosts` and `Cookie::forget` read process-wide state (the
//! registered `AppConfig`, the `SESSION_*` variables), so those tests run
//! alone in a child process, each with the configuration it names.
//!
//! Gated on the `testing` feature, which `Request::for_test_with_headers`
//! and `TestClient` need.
#![cfg(feature = "testing")]

use std::net::IpAddr;

use suprnova::config::{AppConfig, Config, Environment};
use suprnova::http::TrustedProxiesConfig;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    Cookie, FrameworkError, HttpResponse, MiddlewareRegistry, Request, Router, TrustHosts,
};

fn request_with(headers: &[(&str, &str)]) -> Request {
    Request::for_test_with_headers("GET", "/", headers.iter().copied())
}

// ---- Request::ajax ---------------------------------------------------------

/// Symfony's `isXmlHttpRequest` compares the header to `XMLHttpRequest`
/// exactly, so any other spelling is not an AJAX request.
#[test]
fn ajax_is_true_only_for_the_exact_header_value() {
    assert!(request_with(&[("X-Requested-With", "XMLHttpRequest")]).ajax());
    for other in ["xmlhttprequest", "XMLHTTPREQUEST", "XMLHttpRequest2", ""] {
        assert!(
            !request_with(&[("X-Requested-With", other)]).ajax(),
            "{other:?} is not XMLHttpRequest"
        );
    }
    assert!(!request_with(&[]).ajax(), "no header is no AJAX request");
}

// ---- Request::host and the accessors built on it ------------------------

#[test]
fn host_lowercases_and_strips_a_numeric_port() {
    let request = request_with(&[("Host", "Example.COM:8080")]);
    assert_eq!(request.host().as_deref(), Some("example.com"));
    assert_eq!(request.http_host().as_deref(), Some("example.com:8080"));
    assert_eq!(
        request.scheme_and_http_host().as_deref(),
        Some("http://example.com:8080")
    );

    let request = request_with(&[("Host", "[::1]:8080")]);
    assert_eq!(request.host().as_deref(), Some("[::1]"));
}

/// Symfony strips `/:\d+$/` only: a suffix that is not a port stays on the
/// host, where it is a valid host character.
#[test]
fn host_keeps_a_suffix_that_is_not_a_port() {
    let request = request_with(&[("Host", "example.com:abc")]);
    assert_eq!(request.host().as_deref(), Some("example.com:abc"));
}

#[test]
fn host_refuses_a_host_with_a_forbidden_byte() {
    for forged in ["bad<host>", "example.com/evil", "a..b", "user@example.com"] {
        let request = request_with(&[("Host", forged)]);
        assert_eq!(request.host(), None, "{forged:?} is no host");
        assert_eq!(request.http_host(), None, "{forged:?}");
        assert_eq!(request.scheme_and_http_host(), None, "{forged:?}");
        assert!(
            !request.url().contains(forged),
            "the URL built from {forged:?} carries it: {}",
            request.url()
        );
    }
}

/// A forwarded host from a trusted proxy is judged by the same rule.
#[test]
fn a_forwarded_host_is_normalised_and_checked_as_well() {
    let loopback = IpAddr::from([127, 0, 0, 1]);
    let trusted = || TrustedProxiesConfig::with_ips([loopback]);

    let request = request_with(&[("X-Forwarded-Host", "API.Example.com:8443")])
        .with_peer_addr(loopback)
        .with_trusted_proxies(trusted());
    assert_eq!(request.host().as_deref(), Some("api.example.com"));

    let request = request_with(&[("Host", "example.com"), ("X-Forwarded-Host", "bad<host>")])
        .with_peer_addr(loopback)
        .with_trusted_proxies(trusted());
    assert_eq!(
        request.host(),
        None,
        "an invalid forwarded host is refused, not replaced by Host"
    );
}

// ---- TrustHosts ------------------------------------------------------------

/// Run the test `child` (its path in this binary) alone in a child
/// process, with each of `vars` set to its value or, for `None`, removed,
/// and fail unless it ran and passed.
fn run_child(child: &str, vars: &[(&str, Option<&str>)]) {
    let child = {
        let _env = crate::env_lock::lock_env();
        let mut command = crate::own_process::child_command(child);
        for (name, value) in vars {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        command.spawn().expect("spawn the child process")
    };
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
}

/// Register the process-wide `AppConfig` a child test runs under.
fn register_app(environment: Environment, url: &str) {
    Config::register(
        AppConfig::builder()
            .environment(environment)
            .debug(false)
            .url(url)
            .build(),
    );
}

/// A client whose one route answers `reached`, behind `trust` alone.
fn trusted_client(trust: TrustHosts) -> TestClient {
    let router: Router = Router::new()
        .get("/", |_request: Request| async {
            Ok(HttpResponse::text("reached"))
        })
        .into();
    TestClient::new(router, MiddlewareRegistry::new().append(trust))
}

async fn visit(client: &TestClient, host: &str) -> TestResponse {
    client.get("/").header("Host", host).send().await
}

fn assert_reached(response: &TestResponse, host: &str) {
    assert_eq!(response.status(), 200, "{host} is trusted");
    assert_eq!(response.body_text(), "reached", "{host}");
}

fn assert_bad_request(response: &TestResponse, host: &str) {
    assert_eq!(response.status(), 400, "{host} is not trusted");
    assert!(
        response.body_text().contains("Bad request."),
        "{host}: {}",
        response.body_text()
    );
    assert_ne!(
        response.body_text(),
        "reached",
        "{host} reached the handler"
    );
}

#[test]
fn trust_hosts_at_refuses_a_host_no_pattern_matches() {
    run_child(
        "laravel_http_gaps::requests::trust_hosts_at_refuses_a_host_no_pattern_matches_child",
        &[],
    );
}

#[tokio::test]
async fn trust_hosts_at_refuses_a_host_no_pattern_matches_child() {
    if !crate::own_process::is_child() {
        return;
    }
    register_app(Environment::Production, "https://app.test");
    let client = trusted_client(TrustHosts::at([r"^example\.com$"], false).expect("valid"));

    assert_bad_request(&visit(&client, "evil.test").await, "evil.test");
    assert_reached(&visit(&client, "example.com").await, "example.com");
    // Matched without regard to case, as Laravel's patterns are.
    assert_reached(&visit(&client, "EXAMPLE.com").await, "EXAMPLE.com");
    // Anchored: a subdomain is another host.
    assert_bad_request(&visit(&client, "api.example.com").await, "api.example.com");
    // Without subdomains, the APP_URL host is not added.
    assert_bad_request(&visit(&client, "app.test").await, "app.test");
    // An invalid host is refused before any pattern sees it.
    assert_bad_request(&visit(&client, "bad<host>").await, "bad<host>");
}

#[test]
fn trust_hosts_new_trusts_the_app_url_host_and_its_subdomains() {
    run_child(
        "laravel_http_gaps::requests::trust_hosts_new_trusts_the_app_url_host_and_its_subdomains_child",
        &[],
    );
}

#[tokio::test]
async fn trust_hosts_new_trusts_the_app_url_host_and_its_subdomains_child() {
    if !crate::own_process::is_child() {
        return;
    }
    register_app(Environment::Production, "https://example.com");
    let client = trusted_client(TrustHosts::new());

    assert_reached(&visit(&client, "example.com").await, "example.com");
    assert_reached(&visit(&client, "api.example.com").await, "api.example.com");
    assert_reached(
        &visit(&client, "Api.Example.com:8443").await,
        "Api.Example.com:8443",
    );
    assert_bad_request(&visit(&client, "evil.test").await, "evil.test");
    assert_bad_request(&visit(&client, "notexample.com").await, "notexample.com");
    assert_bad_request(
        &visit(&client, "example.com.evil.test").await,
        "example.com.evil.test",
    );
}

#[test]
fn trust_hosts_at_with_subdomains_adds_the_app_url_host() {
    run_child(
        "laravel_http_gaps::requests::trust_hosts_at_with_subdomains_adds_the_app_url_host_child",
        &[],
    );
}

#[tokio::test]
async fn trust_hosts_at_with_subdomains_adds_the_app_url_host_child() {
    if !crate::own_process::is_child() {
        return;
    }
    register_app(Environment::Staging, "https://example.com/billing");
    let client = trusted_client(TrustHosts::at([r"^partner\.test$"], true).expect("valid"));

    assert_reached(&visit(&client, "partner.test").await, "partner.test");
    assert_reached(&visit(&client, "example.com").await, "example.com");
    assert_reached(&visit(&client, "www.example.com").await, "www.example.com");
    assert_bad_request(&visit(&client, "evil.test").await, "evil.test");
}

/// Laravel's `shouldSpecifyTrustedHosts` is false in the local
/// environment: every host reaches the application there.
#[test]
fn trust_hosts_trusts_every_host_in_the_local_environment() {
    run_child(
        "laravel_http_gaps::requests::trust_hosts_trusts_every_host_in_the_local_environment_child",
        &[],
    );
}

#[tokio::test]
async fn trust_hosts_trusts_every_host_in_the_local_environment_child() {
    if !crate::own_process::is_child() {
        return;
    }
    register_app(Environment::Local, "https://example.com");
    let client = trusted_client(TrustHosts::at([r"^example\.com$"], false).expect("valid"));

    assert_reached(&visit(&client, "evil.test").await, "evil.test");
    assert_reached(
        &visit(&client, "192.168.1.20:8000").await,
        "192.168.1.20:8000",
    );
}

#[test]
fn trust_hosts_at_refuses_an_invalid_pattern() {
    let error = TrustHosts::at(["^(unclosed"], false).expect_err("an invalid pattern");
    assert!(error.to_string().contains("^(unclosed"), "{error}");
}

// ---- Cookie::forget --------------------------------------------------------

/// A session store that holds nothing: the deletion-cookie test needs the
/// session middleware's scope, not a stored session.
struct NoSessions;

#[async_trait::async_trait]
impl SessionStore for NoSessions {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }

    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Laravel's cookie jar takes its defaults from `session.path`,
/// `session.domain` and `session.same_site`. Outside a request the
/// environment is the session configuration.
#[test]
fn cookie_forget_takes_its_attributes_from_the_session_environment() {
    run_child(
        "laravel_http_gaps::requests::cookie_forget_takes_its_attributes_from_the_session_environment_child",
        &[
            ("SESSION_PATH", Some("/app")),
            ("SESSION_DOMAIN", Some(".example.com")),
            ("SESSION_SAME_SITE", Some("strict")),
            ("SESSION_COOKIE_PREFIX", None),
        ],
    );
}

#[test]
fn cookie_forget_takes_its_attributes_from_the_session_environment_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let header = Cookie::forget("prefs").to_header_value();
    for attribute in [
        "prefs=",
        "Path=/app",
        "Domain=.example.com",
        "SameSite=Strict",
        "Secure",
        "HttpOnly",
        "Max-Age=0",
    ] {
        assert!(header.contains(attribute), "{attribute} missing: {header}");
    }

    // An explicit path and domain still win; SameSite still follows the
    // session.
    let header =
        Cookie::forget_with("prefs", Some("/admin"), Some("admin.example.com")).to_header_value();
    assert!(header.contains("Path=/admin"), "{header}");
    assert!(header.contains("Domain=admin.example.com"), "{header}");
    assert!(!header.contains("Domain=.example.com"), "{header}");
    assert!(header.contains("SameSite=Strict"), "{header}");

    // With neither given, `forget_with` is `forget`.
    assert_eq!(
        Cookie::forget_with("prefs", None, None).to_header_value(),
        Cookie::forget("prefs").to_header_value()
    );
}

/// Without `SESSION_*` overrides the deletion cookie keeps the framework
/// defaults: the root path, no domain, `SameSite=Lax`.
#[test]
fn cookie_forget_without_session_overrides_keeps_the_defaults() {
    run_child(
        "laravel_http_gaps::requests::cookie_forget_without_session_overrides_keeps_the_defaults_child",
        &[
            ("SESSION_PATH", None),
            ("SESSION_DOMAIN", None),
            ("SESSION_SAME_SITE", None),
            ("SESSION_COOKIE_PREFIX", None),
            ("APP_URL", None),
        ],
    );
}

#[test]
fn cookie_forget_without_session_overrides_keeps_the_defaults_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let header = Cookie::forget("prefs").to_header_value();
    assert!(header.contains("Path=/"), "{header}");
    assert!(!header.contains("Path=/app"), "{header}");
    assert!(!header.contains("Domain="), "{header}");
    assert!(header.contains("SameSite=Lax"), "{header}");
    assert!(header.contains("Secure"), "{header}");
}

/// Under a `__Host-` session cookie prefix an ordinary deletion cookie
/// takes the public root, the path the cookie it deletes was set with.
/// Only a `__Host-` name takes `Path=/` (PFX-007).
#[test]
fn cookie_forget_under_a_host_prefix_follows_the_name_not_the_prefix() {
    run_child(
        "laravel_http_gaps::requests::cookie_forget_under_a_host_prefix_follows_the_name_not_the_prefix_child",
        &[
            ("APP_URL", Some("https://example.test/billing")),
            ("SESSION_PATH", None),
            ("SESSION_DOMAIN", None),
            ("SESSION_SAME_SITE", None),
            ("SESSION_COOKIE_PREFIX", Some("host")),
        ],
    );
}

#[test]
fn cookie_forget_under_a_host_prefix_follows_the_name_not_the_prefix_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let header = Cookie::forget("prefs").to_header_value();
    assert!(header.contains("Path=/billing;"), "{header}");
    assert!(!header.contains("Domain="), "{header}");

    let header = Cookie::forget("__Host-session").to_header_value();
    assert!(header.contains("Path=/;"), "{header}");
    assert!(!header.contains("Domain="), "{header}");

    let header = Cookie::forget_with("prefs", Some("/admin"), None).to_header_value();
    assert!(header.contains("Path=/admin;"), "{header}");
}

/// `SESSION_PATH` sets the deletion path of an ordinary cookie under a
/// `__Host-` session prefix too.
#[test]
fn cookie_forget_under_a_host_prefix_takes_session_path() {
    run_child(
        "laravel_http_gaps::requests::cookie_forget_under_a_host_prefix_takes_session_path_child",
        &[
            ("APP_URL", Some("https://example.test/billing")),
            ("SESSION_PATH", Some("/app")),
            ("SESSION_DOMAIN", None),
            ("SESSION_SAME_SITE", None),
            ("SESSION_COOKIE_PREFIX", Some("host")),
        ],
    );
}

#[test]
fn cookie_forget_under_a_host_prefix_takes_session_path_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let header = Cookie::forget("prefs").to_header_value();
    assert!(header.contains("Path=/app;"), "{header}");
    assert!(!header.contains("Path=/billing"), "{header}");
}

/// Inside a request `SessionMiddleware` serves, its configuration is the
/// one the deletion cookie follows, so a response's `without_cookie`
/// clears a cookie the application set with the session's scope.
#[tokio::test]
async fn cookie_forget_inside_a_session_request_follows_its_configuration() {
    suprnova::testing::install_test_encryption_key();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.cookie_path = "/shop".to_string();
    config.cookie_domain = Some(".shop.test".to_string());
    config.cookie_same_site = "None".to_string();
    let router: Router = Router::new()
        .get("/forget", |_request: Request| async {
            Ok(HttpResponse::text(
                Cookie::forget("prefs").to_header_value(),
            ))
        })
        .get("/forget-with", |_request: Request| async {
            Ok(HttpResponse::text(
                Cookie::forget_with("prefs", Some("/"), None).to_header_value(),
            ))
        })
        .into();
    let client = TestClient::new(
        router,
        MiddlewareRegistry::new().append(SessionMiddleware::with_store(
            config,
            std::sync::Arc::new(NoSessions),
        )),
    );

    let header = client.get("/forget").send().await.body_text();
    for attribute in ["Path=/shop", "Domain=.shop.test", "SameSite=None", "Secure"] {
        assert!(header.contains(attribute), "{attribute} missing: {header}");
    }

    let header = client.get("/forget-with").send().await.body_text();
    assert!(
        header.contains("Path=/;"),
        "the explicit path wins: {header}"
    );
    assert!(
        header.contains("Domain=.shop.test"),
        "the domain still follows the session: {header}"
    );
}
