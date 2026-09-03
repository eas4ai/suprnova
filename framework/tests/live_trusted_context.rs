use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::http::HttpResponse;
use suprnova::live::testing::{
    LiveContextHarness, LiveSecurityCheck, LiveSecurityDisposition, LiveTestOperation,
    LiveTestRoutePolicy, complete_live_route_policy_for_test, inspect_request_attestation,
    prepare_live_request_for_test, prepare_live_request_until_for_test,
    record_live_security_not_required_for_test, record_live_security_pass_for_test,
    register_live_route_for_test, remove_live_security_check_for_test,
    request_cancellation_for_test, same_request_identity,
};
use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};
use suprnova::session::{SessionConfig, SessionData, SessionStore};
use suprnova::{
    App, AuthMiddleware, Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, Middleware,
    MiddlewareRegistry, Next, OriginPolicy, Request, Response, Router, SessionMiddleware,
    async_trait, handle_request,
};

fn capture_attestation(
    captured: Arc<Mutex<Option<suprnova::live::testing::LiveSecurityReport>>>,
) -> Next {
    Arc::new(move |request| {
        let captured = Arc::clone(&captured);
        Box::pin(async move {
            *captured.lock().expect("capture lock") = Some(inspect_request_attestation(&request));
            Ok(HttpResponse::text("reached"))
        })
    })
}

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn anonymous_policy() -> LiveTestRoutePolicy {
    LiveTestRoutePolicy {
        trusted_internal_origin: false,
        stateless_csrf: false,
        stateless_session: true,
        anonymous_principal: true,
        tenantless: true,
        direct_peer: false,
        upstream_rate_limit: true,
        no_additional_middleware: true,
    }
}

fn complete_anonymous_request(operation: LiveTestOperation) -> Request {
    let mut request = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/control").with_route_pattern("/__live/v1/control"),
        operation,
    );
    assert!(record_live_security_pass_for_test(
        &mut request,
        LiveSecurityCheck::Origin,
        None,
    ));
    if matches!(
        operation,
        LiveTestOperation::Action | LiveTestOperation::Upload
    ) {
        assert!(record_live_security_pass_for_test(
            &mut request,
            LiveSecurityCheck::Csrf,
            None,
        ));
    } else {
        assert!(record_live_security_not_required_for_test(
            &mut request,
            LiveSecurityCheck::Csrf,
        ));
    }
    complete_live_route_policy_for_test(&mut request, anonymous_policy());
    request
}

async fn dispatch_one(router: Router, request: hyper::Request<Full<Bytes>>) {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });

    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect test request");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(request)
        .await
        .expect("send test request");
    assert_eq!(response.status(), 200);
    let _ = response.into_body().collect().await.expect("response body");
}

struct CleanSessionStore;

#[async_trait]
impl SessionStore for CleanSessionStore {
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

fn live_session_middleware() -> SessionMiddleware {
    ensure_crypt();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    SessionMiddleware::with_store(config, Arc::new(CleanSessionStore))
}

#[test]
fn a_fresh_request_has_unique_identity_and_no_implicit_security_proof() {
    let first =
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action");
    let second =
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action");

    assert!(!same_request_identity(&first, &second));

    let report = inspect_request_attestation(&first);
    assert!(!report.is_complete());
    assert_eq!(
        report.missing_checks(),
        &[
            LiveSecurityCheck::Origin,
            LiveSecurityCheck::Csrf,
            LiveSecurityCheck::Session,
            LiveSecurityCheck::Principal,
            LiveSecurityCheck::Tenant,
            LiveSecurityCheck::Proxy,
            LiveSecurityCheck::RateLimit,
            LiveSecurityCheck::Middleware,
        ]
    );
    assert_eq!(format!("{report:?}"), "<LiveSecurityReport:redacted>");
}

#[tokio::test]
async fn registered_live_route_prepares_before_owner_middleware_and_completes_policy() {
    ensure_crypt();
    App::init();
    let captured = Arc::new(Mutex::new(None));
    let captured_in_handler = Arc::clone(&captured);
    let mut router: Router = Router::new()
        .post("/__live/v1/sse/control", move |request: Request| {
            let captured = Arc::clone(&captured_in_handler);
            async move {
                *captured.lock().expect("capture lock") =
                    Some(inspect_request_attestation(&request));
                Ok(HttpResponse::text("ok"))
            }
        })
        .middleware(CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly))
        .into();
    register_live_route_for_test(
        &mut router,
        hyper::Method::POST,
        "/__live/v1/sse/control",
        LiveTestOperation::SseControl,
        anonymous_policy(),
    )
    .expect("register Live route metadata");

    let request = hyper::Request::builder()
        .method("POST")
        .uri("/__live/v1/sse/control")
        .header("host", "localhost")
        .header("sec-fetch-site", "same-origin")
        .body(Full::new(Bytes::new()))
        .expect("test request");
    dispatch_one(router, request).await;

    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("Live handler report");
    assert!(
        report.is_complete(),
        "missing Live checks: {:?}",
        report.missing_checks()
    );
    assert!(report.order_is_valid());
    assert_eq!(report.present_count(), 8);
    assert_eq!(
        report.disposition(LiveSecurityCheck::Origin),
        Some(LiveSecurityDisposition::Passed)
    );
    assert_eq!(
        report.disposition(LiveSecurityCheck::Csrf),
        Some(LiveSecurityDisposition::NotRequired)
    );
}

#[test]
fn production_context_validator_accepts_all_four_complete_operation_classes() {
    ensure_crypt();
    let harness = LiveContextHarness::anonymous().expect("context harness");
    for operation in [
        LiveTestOperation::Action,
        LiveTestOperation::Upload,
        LiveTestOperation::SseControl,
        LiveTestOperation::WebSocketHandshake,
    ] {
        let request = complete_anonymous_request(operation);
        harness
            .validate(&request)
            .unwrap_or_else(|error| panic!("{operation:?} context was rejected: {error}"));
    }
}

#[test]
fn production_context_validator_rejects_the_full_hostile_matrix_for_every_operation() {
    ensure_crypt();
    let harness = LiveContextHarness::anonymous().expect("context harness");
    for operation in [
        LiveTestOperation::Action,
        LiveTestOperation::Upload,
        LiveTestOperation::SseControl,
        LiveTestOperation::WebSocketHandshake,
    ] {
        for check in [
            LiveSecurityCheck::Origin,
            LiveSecurityCheck::Csrf,
            LiveSecurityCheck::Session,
            LiveSecurityCheck::Principal,
            LiveSecurityCheck::Tenant,
            LiveSecurityCheck::Proxy,
            LiveSecurityCheck::RateLimit,
            LiveSecurityCheck::Middleware,
        ] {
            let mut omitted = complete_anonymous_request(operation);
            remove_live_security_check_for_test(&mut omitted, check);
            assert!(
                harness.validate(&omitted).is_err(),
                "{operation:?} must reject omitted {check:?} evidence"
            );
        }

        let mut wrong_order = prepare_live_request_for_test(
            Request::for_test("POST", "/__live/v1/control")
                .with_route_pattern("/__live/v1/control"),
            operation,
        );
        assert!(record_live_security_pass_for_test(
            &mut wrong_order,
            LiveSecurityCheck::Origin,
            None,
        ));
        assert!(!record_live_security_pass_for_test(
            &mut wrong_order,
            LiveSecurityCheck::Session,
            Some(b"late-session"),
        ));
        assert!(
            harness.validate(&wrong_order).is_err(),
            "{operation:?} must reject wrong-order evidence"
        );

        let bypassed = prepare_live_request_for_test(
            Request::for_test_with_headers(
                "POST",
                "/__live/v1/control",
                [
                    ("origin", "https://attacker.invalid"),
                    ("x-csrf-token", "forged"),
                    ("authorization", "Bearer forged"),
                    ("x-tenant-id", "forged"),
                ],
            )
            .with_route_pattern("/__live/v1/control"),
            operation,
        );
        assert!(
            harness.validate(&bypassed).is_err(),
            "{operation:?} must reject exception/bypass branches that only supplied headers"
        );

        let mut short_circuited = prepare_live_request_for_test(
            Request::for_test("POST", "/__live/v1/control")
                .with_route_pattern("/__live/v1/control"),
            operation,
        );
        assert!(record_live_security_pass_for_test(
            &mut short_circuited,
            LiveSecurityCheck::Origin,
            None,
        ));
        assert!(
            harness.validate(&short_circuited).is_err(),
            "{operation:?} must reject a middleware short-circuit before downstream evidence"
        );

        let mut expired = prepare_live_request_until_for_test(
            Request::for_test("POST", "/__live/v1/control")
                .with_route_pattern("/__live/v1/control"),
            operation,
            1,
        );
        assert!(record_live_security_pass_for_test(
            &mut expired,
            LiveSecurityCheck::Origin,
            None,
        ));
        if matches!(
            operation,
            LiveTestOperation::Action | LiveTestOperation::Upload
        ) {
            assert!(record_live_security_pass_for_test(
                &mut expired,
                LiveSecurityCheck::Csrf,
                None,
            ));
        } else {
            assert!(record_live_security_not_required_for_test(
                &mut expired,
                LiveSecurityCheck::Csrf,
            ));
        }
        complete_live_route_policy_for_test(&mut expired, anonymous_policy());
        assert!(
            harness.validate(&expired).is_err(),
            "{operation:?} must reject stale request reuse"
        );
    }
}

#[test]
fn dropping_a_prepared_request_cancels_host_owned_live_work() {
    let request = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let cancellation =
        request_cancellation_for_test(&request).expect("prepared Live request owns cancellation");
    assert!(!cancellation.is_canceled());
    drop(request);
    assert!(cancellation.is_canceled());
}

#[test]
fn attacker_headers_do_not_mint_framework_security_evidence() {
    let request = Request::for_test_with_headers(
        "POST",
        "/__live/v1/action",
        [
            ("origin", "https://example.test"),
            ("x-csrf-token", "attacker-controlled"),
            ("authorization", "Bearer attacker-controlled"),
            ("x-tenant-id", "attacker-controlled"),
            ("x-forwarded-for", "203.0.113.7"),
            ("x-live-rate-limit", "passed"),
        ],
    )
    .with_route_pattern("/__live/v1/action");

    let report = inspect_request_attestation(&request);
    assert!(!report.is_complete());
    assert_eq!(report.present_count(), 0);
}

/// The shipped Live runtime carries no session token: the configured origin
/// proof is the CSRF proof for a Live action, exactly as it is for every other
/// same-origin state change under an origin-verifying policy.
#[tokio::test]
async fn live_actions_accept_the_configured_origin_proof_without_a_token() {
    let request = prepare_live_request_for_test(
        Request::for_test_with_headers(
            "POST",
            "/__live/v1/action",
            [("sec-fetch-site", "same-origin")],
        )
        .with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let middleware = CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly);
    let session = suprnova::session::new_session_slot_for_test();

    let response = suprnova::session::session_scope_for_test(
        session,
        middleware.handle(request, capture_attestation(Arc::clone(&captured))),
    )
    .await;
    assert!(response.is_ok());

    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("downstream request report");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Origin),
        Some(LiveSecurityDisposition::Passed)
    );
    assert_eq!(
        report.disposition(LiveSecurityCheck::Csrf),
        Some(LiveSecurityDisposition::NotRequired)
    );
}

/// Without an accepted origin proof the token path still decides: a valid
/// token passes and records the CSRF fact, and a missing token is refused.
#[tokio::test]
async fn live_actions_without_an_origin_proof_fall_back_to_the_token() {
    let with_token = prepare_live_request_for_test(
        Request::for_test_with_headers(
            "POST",
            "/__live/v1/action",
            [("x-csrf-token", "test_csrf_token")],
        )
        .with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let middleware = CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly);
    let session = suprnova::session::new_session_slot_for_test();
    let response = suprnova::session::session_scope_for_test(
        session,
        middleware.handle(with_token, capture_attestation(Arc::clone(&captured))),
    )
    .await;
    assert!(response.is_ok());
    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("downstream request report");
    assert_eq!(report.disposition(LiveSecurityCheck::Origin), None);
    assert_eq!(
        report.disposition(LiveSecurityCheck::Csrf),
        Some(LiveSecurityDisposition::Passed)
    );

    let without_token = prepare_live_request_for_test(
        Request::for_test_with_headers(
            "POST",
            "/__live/v1/action",
            [("sec-fetch-site", "cross-site")],
        )
        .with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let session = suprnova::session::new_session_slot_for_test();
    let response = suprnova::session::session_scope_for_test(
        session,
        middleware.handle(without_token, capture_attestation(Arc::clone(&captured))),
    )
    .await;
    let status = match response {
        Err(rejected) => rejected.status_code(),
        Ok(_) => panic!("a cross-site Live action without a token must be refused"),
    };
    assert_eq!(status, 419);
    assert!(
        captured.lock().expect("capture lock").is_none(),
        "the handler never ran"
    );
}

#[tokio::test]
async fn live_sse_control_records_csrf_as_explicitly_not_required() {
    let request = prepare_live_request_for_test(
        Request::for_test_with_headers(
            "POST",
            "/__live/v1/sse/control",
            [("sec-fetch-site", "same-origin")],
        )
        .with_route_pattern("/__live/v1/sse/control"),
        LiveTestOperation::SseControl,
    );
    let captured = Arc::new(Mutex::new(None));
    let middleware = CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly);

    let response = middleware
        .handle(request, capture_attestation(Arc::clone(&captured)))
        .await;
    assert!(response.is_ok());

    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("downstream request report");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Origin),
        Some(LiveSecurityDisposition::Passed)
    );
    assert_eq!(
        report.disposition(LiveSecurityCheck::Csrf),
        Some(LiveSecurityDisposition::NotRequired)
    );
}

struct FailingLimiter;

#[async_trait]
impl RateLimiterDriver for FailingLimiter {
    async fn try_acquire(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal("test limiter unavailable"))
    }

    async fn retry_after(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        Ok(None)
    }
}

struct FixedTenant(Option<&'static str>);

#[async_trait]
impl LiveTenantResolver for FixedTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(self.0.map(str::to_owned))
    }
}

async fn drive_rate_limit(
    limiter: Arc<dyn RateLimiterDriver>,
    policy: BackendErrorPolicy,
) -> (Response, suprnova::live::testing::LiveSecurityReport) {
    let middleware = RateLimitMiddleware::new(
        limiter,
        SlidingWindowConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
        },
        |_| "live-test".to_string(),
    )
    .on_backend_error(policy);
    let request = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let response = middleware
        .handle(request, capture_attestation(Arc::clone(&captured)))
        .await;
    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("fail-open and admitted requests reach downstream");
    (response, report)
}

#[tokio::test]
async fn only_a_successful_rate_decision_mints_rate_evidence() {
    let (admitted, admitted_report) = drive_rate_limit(
        Arc::new(InMemoryRateLimiter::new()),
        BackendErrorPolicy::FailClosed,
    )
    .await;
    assert!(admitted.is_ok());
    assert_eq!(
        admitted_report.disposition(LiveSecurityCheck::RateLimit),
        Some(LiveSecurityDisposition::Passed)
    );

    let (failed_open, failed_open_report) =
        drive_rate_limit(Arc::new(FailingLimiter), BackendErrorPolicy::FailOpen).await;
    assert!(
        failed_open.is_ok(),
        "ordinary fail-open behavior is retained"
    );
    assert_eq!(
        failed_open_report.disposition(LiveSecurityCheck::RateLimit),
        None,
        "backend failure cannot masquerade as rate admission"
    );
}

#[tokio::test]
async fn successful_session_resolution_mints_request_bound_session_evidence() {
    let request = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));

    let response = live_session_middleware()
        .handle(request, capture_attestation(Arc::clone(&captured)))
        .await;
    assert!(response.is_ok());

    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("session-admitted request reaches downstream");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Session),
        Some(LiveSecurityDisposition::Passed)
    );
}

#[tokio::test]
async fn only_authenticated_middleware_mints_principal_evidence() {
    let request = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let auth_next: Next = {
        let captured = Arc::clone(&captured);
        Arc::new(move |request| {
            let captured = Arc::clone(&captured);
            Box::pin(async move {
                suprnova::session::set_auth_user("principal-42");
                AuthMiddleware::new()
                    .handle(request, capture_attestation(captured))
                    .await
            })
        })
    };

    let response = live_session_middleware().handle(request, auth_next).await;
    assert!(response.is_ok());

    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("authenticated request reaches downstream");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Session),
        Some(LiveSecurityDisposition::Passed)
    );
    assert_eq!(
        report.disposition(LiveSecurityCheck::Principal),
        Some(LiveSecurityDisposition::Passed)
    );

    let anonymous = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let next: Next = {
        let reached = Arc::clone(&reached);
        Arc::new(move |_request| {
            reached.store(true, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { Ok(HttpResponse::text("must not run")) })
        })
    };
    let response = live_session_middleware()
        .handle(
            anonymous,
            Arc::new(move |request| {
                let next = Arc::clone(&next);
                Box::pin(async move { AuthMiddleware::new().handle(request, next).await })
            }),
        )
        .await;
    let rejection = match response {
        Ok(_) => panic!("anonymous request must be rejected"),
        Err(response) => response,
    };
    assert_eq!(rejection.status_code(), 401);
    assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn tenant_and_proxy_evidence_come_only_from_framework_owned_resolution() {
    let request = prepare_live_request_for_test(
        Request::for_test_with_headers(
            "POST",
            "/__live/v1/action",
            [
                ("x-tenant-id", "forged-tenant"),
                ("x-forwarded-for", "203.0.113.8"),
            ],
        )
        .with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let prepared = inspect_request_attestation(&request);
    assert_eq!(
        prepared.disposition(LiveSecurityCheck::Proxy),
        Some(LiveSecurityDisposition::NotRequired),
        "an untrusted proxy header is ignored under direct-peer policy"
    );
    assert_eq!(prepared.disposition(LiveSecurityCheck::Tenant), None);

    let captured = Arc::new(Mutex::new(None));
    let response = LiveTenantMiddleware::new(Arc::new(FixedTenant(Some("tenant-7"))))
        .handle(request, capture_attestation(Arc::clone(&captured)))
        .await;
    assert!(response.is_ok());
    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("tenant-resolved request reaches downstream");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Tenant),
        Some(LiveSecurityDisposition::Passed)
    );

    let tenantless = prepare_live_request_for_test(
        Request::for_test("POST", "/__live/v1/action").with_route_pattern("/__live/v1/action"),
        LiveTestOperation::Action,
    );
    let captured = Arc::new(Mutex::new(None));
    let response = LiveTenantMiddleware::new(Arc::new(FixedTenant(None)))
        .handle(tenantless, capture_attestation(Arc::clone(&captured)))
        .await;
    assert!(response.is_ok());
    let report = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("tenantless request reaches downstream");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Tenant),
        Some(LiveSecurityDisposition::NotRequired)
    );
}
