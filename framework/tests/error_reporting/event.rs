use super::drivers::Capture;
use serial_test::serial;
use std::sync::Arc;
use suprnova::testing::TestClient;
use suprnova::{
    App, Context, Exceptions, FrameworkError, HttpResponse, Log, MiddlewareRegistry, Reporter,
    Reporting, ReportingConfig, ReportingDriver, Router,
};

#[test]
#[serial]
fn report_003_event_timestamp_follows_the_framework_clock() {
    let _clock = suprnova::testing::TestClock::travel_to(
        "2000-01-01T00:00:00Z".parse().expect("fixed test time"),
    );
    let event = suprnova::ReportingEvent::from_error_with_config(
        &FrameworkError::internal("clocked report"),
        &ReportingConfig::default(),
    );
    assert_eq!(event.payload["timestamp"].as_f64(), Some(946_684_800.0));
}

#[test]
#[serial]
fn report_003_release_defaults_to_the_application_version_registered_at_boot() {
    suprnova::boot::set_default_build_id("reporting-application@8.7.6");
    let event = suprnova::ReportingEvent::from_error_with_config(
        &FrameworkError::internal("application release"),
        &ReportingConfig::default(),
    );
    assert_eq!(event.payload["release"], "reporting-application@8.7.6");
}

#[tokio::test]
#[serial]
async fn report_003_production_event_has_context_frames_and_request_breadcrumbs() {
    let _clock = suprnova::testing::TestClock::travel_to(
        "2000-01-01T00:00:00Z".parse().expect("fixed test time"),
    );
    Exceptions::reset();
    let subscriber = suprnova::logging::build_subscriber(suprnova::LogConfig::default())
        .expect("logging subscriber");
    let _subscriber = tracing::subscriber::set_default(subscriber);
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Null)
            .release("shop@2.3.4")
            .environment("production"),
    )
    .expect("config");
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    let client = TestClient::new(
        Router::new().get("/orders/{id}", |_req: suprnova::Request| async {
            Log::warning("payment retry");
            Err::<HttpResponse, _>(HttpResponse::from(FrameworkError::from_external(
                std::io::Error::other("processor unavailable"),
            )))
        }),
        MiddlewareRegistry::new(),
    );
    client
        .get("https://shop.example/orders/123?api_key=private-key&currency=USD")
        .header("authorization", "Bearer private-auth")
        .header("cookie", "session=private-session")
        .header("x-request-id", "report-003")
        .send()
        .await
        .assert_status(500);
    let events = sink.0.lock().expect("capture");
    assert_eq!(events.len(), 1);
    let event = &events[0].payload;
    assert_eq!(event["release"], "shop@2.3.4");
    assert_eq!(event["environment"], "production");
    assert!(!event["server_name"].as_str().expect("hostname").is_empty());
    assert_eq!(event["request"]["method"], "GET");
    assert_eq!(event["transaction"], "/orders/{id}");
    assert_eq!(event["tags"]["request_id"], "report-003");
    assert!(
        event["exception"]["values"][0]["type"]
            .as_str()
            .expect("type")
            .contains("io")
    );
    assert!(
        !event["exception"]["values"][0]["stacktrace"]["frames"]
            .as_array()
            .expect("frames")
            .is_empty()
    );
    assert!(event["breadcrumbs"].to_string().contains("payment retry"));
    assert_eq!(
        event["breadcrumbs"]["values"][0]["timestamp"].as_f64(),
        Some(946_684_800.0),
    );
    let encoded = event.to_string();
    for secret in ["private-key", "private-auth", "private-session"] {
        assert!(!encoded.contains(secret), "leaked {secret}");
    }
    assert!(event["user"]["email"].is_null());
    assert!(event["user"]["ip_address"].is_null());
}

#[tokio::test]
#[serial]
async fn report_004_hidden_context_and_secret_fields_never_reach_the_driver() {
    Exceptions::reset();
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    Context::scope(Default::default(), async {
        Context::add("checkout",serde_json::json!({"password":"private-password","csrf":"private-csrf","session_id":"private-id","safe":"visible"}));
        Context::hidden_add("private", "private-hidden");
        Exceptions::report(&FrameworkError::internal("failed?TOKEN=private-token"));
    }).await;
    let events = sink.0.lock().expect("capture");
    let encoded = events[0].payload.to_string();
    for secret in [
        "private-password",
        "private-csrf",
        "private-id",
        "private-hidden",
        "private-token",
    ] {
        assert!(!encoded.contains(secret), "leaked {secret}");
    }
    assert!(encoded.contains("visible"));
    assert!(events[0].payload["request"]["data"].is_null());
}

/// A signed-in user of the test's `admin` guard.
#[derive(Clone)]
struct Admin(String);

impl suprnova::Authenticatable for Admin {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// A provider that knows nobody: the test signs its user in directly.
struct Nobody;

#[async_trait::async_trait]
impl suprnova::UserProvider for Nobody {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn suprnova::Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

#[tokio::test]
#[serial]
async fn report_003_a_named_guard_route_reports_its_own_user() {
    suprnova::testing::TestContainer::scope(async {
        Exceptions::reset();
        let config = suprnova::AuthConfig::new("web")
            .guard("admin", suprnova::GuardConfig::session("users"));
        suprnova::testing::TestContainer::singleton(suprnova::AuthManager::new(config));
        suprnova::Auth::register_provider("users", Arc::new(Nobody)).expect("provider");
        let sink = Arc::new(Capture::default());
        App::bind::<dyn Reporter>(sink.clone());
        let client = TestClient::new(
            Router::new()
                .get("/admin/fail", |_req: suprnova::Request| async {
                    Err::<HttpResponse, _>(HttpResponse::from(FrameworkError::internal(
                        "admin failure",
                    )))
                })
                .middleware(suprnova::AuthMiddleware::new().for_guard("admin")),
            MiddlewareRegistry::new(),
        )
        .acting_as_with_guard(&Admin("9".to_owned()), "admin");
        client.get("/admin/fail").send().await.assert_status(500);
        let events = sink.0.lock().expect("capture");
        assert_eq!(events.len(), 1);
        // The admin guard's user 9, never the default guard's user 9.
        assert_eq!(events[0].payload["user"]["id"], "admin:9");
    })
    .await;
}
