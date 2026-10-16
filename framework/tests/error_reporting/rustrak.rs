use serde_json::{Value, json};
use std::time::{Duration, Instant};
use suprnova::testing::TestClient;
use suprnova::{
    Exceptions, FrameworkError, Log, LogLevel, MiddlewareRegistry, Reporting, ReportingConfig,
    ReportingDriver, Router,
};

struct Tracker {
    client: reqwest::Client,
    base: String,
    cookie: String,
    project: Value,
}
impl Tracker {
    async fn new() -> Self {
        let setting = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required for the real Rustrak reporting gate")
            })
        };
        let base = setting("RUSTRAK_TEST_URL")
            .trim_end_matches('/')
            .to_string();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Rustrak API client");
        let response = client.post(format!("{base}/auth/login")).json(&json!({"email":setting("RUSTRAK_TEST_USER"),"password":setting("RUSTRAK_TEST_PASSWORD")})).send().await.expect("Rustrak login request");
        assert!(
            response.status().is_success(),
            "Rustrak login failed: {}",
            response.status()
        );
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|h| h.to_str().ok())
            .filter_map(|h| h.split(';').next())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(
            !cookie.is_empty(),
            "Rustrak login did not issue a session cookie"
        );
        let project: Value = client
            .post(format!("{base}/api/projects"))
            .header("cookie", &cookie)
            .json(
                &json!({"name":format!("Suprnova gate {}",uuid::Uuid::new_v4()),"platform":"rust"}),
            )
            .send()
            .await
            .expect("create Rustrak project")
            .error_for_status()
            .expect("Rustrak project creation status")
            .json()
            .await
            .expect("project response");
        assert!(
            project["dsn"].is_string(),
            "Rustrak project response has no DSN"
        );
        Self {
            client,
            base,
            cookie,
            project,
        }
    }
    fn dsn(&self) -> String {
        self.project["dsn"]
            .as_str()
            .expect("project DSN")
            .to_owned()
    }
    async fn get(&self, path: &str) -> Value {
        self.client
            .get(format!(
                "{}/api/projects/{}/{}",
                self.base,
                self.project["id"].as_i64().expect("project id"),
                path
            ))
            .header("cookie", &self.cookie)
            .send()
            .await
            .expect("Rustrak API read")
            .error_for_status()
            .expect("Rustrak API status")
            .json()
            .await
            .expect("Rustrak JSON")
    }
    async fn event(&self, marker: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let issues = self.get("issues").await;
            for issue in issues["items"].as_array().expect("issues list") {
                let issue_id = issue["id"].as_str().expect("issue id");
                let events = self.get(&format!("issues/{issue_id}/events")).await;
                for event in events["items"].as_array().expect("events list") {
                    let event_id = event["id"].as_str().expect("event id");
                    let detail = self
                        .get(&format!("issues/{issue_id}/events/{event_id}"))
                        .await;
                    if detail["data"].to_string().contains(marker) {
                        return detail["data"].clone();
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "Rustrak never received expected event {marker}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn remove(self) {
        self.client
            .delete(format!(
                "{}/api/projects/{}",
                self.base,
                self.project["id"].as_i64().expect("id")
            ))
            .header("cookie", self.cookie)
            .send()
            .await
            .expect("remove test project")
            .error_for_status()
            .expect("remove project status");
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs standing Rustrak and RUSTRAK_TEST_URL, RUSTRAK_TEST_USER, RUSTRAK_TEST_PASSWORD"]
async fn report_010_error_rustrak() {
    let tracker = Tracker::new().await;
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Sentry)
            .dsn(tracker.dsn()),
    )
    .expect("reporting configuration");
    Exceptions::report(&FrameworkError::from_external(std::io::Error::other(
        "report-010-error",
    )));
    assert!(Reporting::flush(Duration::from_secs(5)).expect("flush"));
    let event = tracker.event("report-010-error").await;
    assert!(event["exception"]["values"].is_array());
    tracker.remove().await;
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs standing Rustrak and RUSTRAK_TEST_URL, RUSTRAK_TEST_USER, RUSTRAK_TEST_PASSWORD"]
async fn report_010_panic_rustrak() {
    let tracker = Tracker::new().await;
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Sentry)
            .dsn(tracker.dsn()),
    )
    .expect("reporting configuration");
    async fn panicking(_: suprnova::Request) -> suprnova::Response {
        panic!("report-010-panic");
    }
    let client = TestClient::new(
        Router::new().get("/panic", panicking),
        MiddlewareRegistry::new(),
    );
    client.get("/panic").send().await.assert_status(500);
    assert!(Reporting::flush(Duration::from_secs(5)).expect("flush"));
    let event = tracker.event("report-010-panic").await;
    assert_eq!(event["exception"]["values"][0]["type"], "panic");
    tracker.remove().await;
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "needs standing Rustrak and RUSTRAK_TEST_URL, RUSTRAK_TEST_USER, RUSTRAK_TEST_PASSWORD"]
async fn report_010_log_rustrak() {
    let tracker = Tracker::new().await;
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Sentry)
            .dsn(tracker.dsn())
            .logs(Some(LogLevel::Warning)),
    )
    .expect("reporting configuration");
    let subscriber = suprnova::logging::build_subscriber(suprnova::LogConfig::default())
        .expect("logging subscriber");
    let _subscriber = tracing::subscriber::set_default(subscriber);
    Log::warning("report-010-log");
    assert!(Reporting::flush(Duration::from_secs(5)).expect("flush"));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let logs = tracker.get("logs").await;
        if logs["items"]
            .as_array()
            .expect("logs list")
            .iter()
            .any(|log| log["body"] == "report-010-log" && log["level"] == "warn")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Rustrak never received the structured log"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tracker.remove().await;
}
