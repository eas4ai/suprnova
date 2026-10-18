use super::delivery::receiver;
use serial_test::serial;
use std::time::Duration;
use suprnova::testing::TestClient;
use suprnova::{
    HttpResponse, Log, LogLevel, MiddlewareRegistry, Reporting, ReportingConfig, Router,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct BreadcrumbJob;

#[async_trait::async_trait]
impl suprnova::Job for BreadcrumbJob {
    fn job_name() -> &'static str {
        "reporting::BreadcrumbJob"
    }

    async fn handle(self) -> Result<(), suprnova::FrameworkError> {
        Log::warning("job-owned breadcrumb");
        suprnova::report(&suprnova::FrameworkError::internal("job-owned event"))
    }
}

#[tokio::test]
#[serial]
async fn report_006_inline_jobs_have_their_own_transaction_and_breadcrumbs() {
    use std::sync::Arc;
    use suprnova::queue::{Queue, SyncQueueDriver};
    use suprnova::{App, Exceptions, Reporter, ReportingDriver};

    Exceptions::reset();
    let subscriber = suprnova::logging::build_subscriber(suprnova::LogConfig::default())
        .expect("logging subscriber");
    let _subscriber = tracing::subscriber::set_default(subscriber);
    Reporting::configure(
        ReportingConfig::default()
            .driver(ReportingDriver::Null)
            .traces_sample_rate(1.0),
    )
    .expect("configure");
    let events = Arc::new(super::drivers::Capture::default());
    App::bind::<dyn Reporter>(events.clone());
    Queue::set_driver(Arc::new(SyncQueueDriver::new()));
    suprnova::queue::worker::register_job::<BreadcrumbJob>();
    let client = TestClient::new(
        Router::new().get("/dispatch", |_: suprnova::Request| async {
            Log::warning("request-only breadcrumb");
            Queue::push(BreadcrumbJob).await?;
            Ok::<_, HttpResponse>(HttpResponse::text("ok"))
        }),
        MiddlewareRegistry::new(),
    );
    client.get("/dispatch").send().await.assert_ok();
    let events = events.0.lock().expect("captured events");
    let event = events
        .iter()
        .find(|event| event.kind == suprnova::ReportingKind::Event)
        .expect("job event");
    assert!(
        event.payload["breadcrumbs"]
            .to_string()
            .contains("job-owned breadcrumb")
    );
    assert!(
        !event.payload["breadcrumbs"]
            .to_string()
            .contains("request-only breadcrumb")
    );
    assert!(event.payload["request"].is_null());
    let transactions: Vec<_> = events
        .iter()
        .filter(|event| event.kind == suprnova::ReportingKind::Transaction)
        .collect();
    assert_eq!(transactions.len(), 2, "one job and one request transaction");
    let job = transactions
        .iter()
        .find(|event| event.payload["contexts"]["trace"]["op"] == "queue.job")
        .expect("sampled job transaction");
    assert_eq!(job.payload["transaction"], "reporting::BreadcrumbJob");
    let request = transactions
        .iter()
        .find(|event| event.payload["contexts"]["trace"]["op"] == "http.server")
        .expect("sampled request transaction");
    assert_ne!(
        job.payload["contexts"]["trace"]["trace_id"],
        request.payload["contexts"]["trace"]["trace_id"]
    );
}

#[tokio::test]
#[serial]
async fn report_006_logs_and_transactions_are_off_by_default() {
    let subscriber = suprnova::logging::build_subscriber(suprnova::LogConfig::default())
        .expect("logging subscriber");
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let (dsn, received) = receiver();
    Reporting::configure(ReportingConfig::default().dsn(dsn)).expect("configure");
    Log::error("default log is local");
    let client = TestClient::new(
        Router::new().get("/off", |_: suprnova::Request| async {
            Ok(HttpResponse::text("ok"))
        }),
        MiddlewareRegistry::new(),
    );
    client.get("/off").send().await.assert_ok();
    assert!(Reporting::flush(Duration::from_secs(1)).expect("flush"));
    assert!(received.try_recv().is_err());
}

#[tokio::test]
#[serial]
async fn report_006_opt_in_sends_only_selected_logs_and_a_request_transaction() {
    let subscriber = suprnova::logging::build_subscriber(suprnova::LogConfig::default())
        .expect("logging subscriber");
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let (dsn, received) = receiver();
    Reporting::configure(
        ReportingConfig::default()
            .dsn(dsn)
            .logs(Some(LogLevel::Warning))
            .traces_sample_rate(1.0),
    )
    .expect("configure");
    let client = TestClient::new(
        Router::new().get("/traced", |_: suprnova::Request| async {
            Log::info("not selected");
            Log::warning("selected log");
            Ok(HttpResponse::text("ok"))
        }),
        MiddlewareRegistry::new(),
    );
    client.get("/traced").send().await.assert_ok();
    assert!(Reporting::flush(Duration::from_secs(3)).expect("flush"));
    let first = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("first envelope"),
    )
    .expect("UTF-8");
    let second = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("second envelope"),
    )
    .expect("UTF-8");
    let joined = format!("{first}\n{second}");
    assert!(joined.contains("selected log"));
    assert!(!joined.contains("not selected"));
    assert!(joined.contains("\"type\":\"log\""));
    assert!(joined.contains("\"type\":\"transaction\""));
    assert!(joined.contains("/traced"));
    assert!(received.try_recv().is_err());
}
