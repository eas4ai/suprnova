use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serial_test::serial;
use suprnova::schedule::Schedule;
use suprnova::supervisor::{RestartPolicy, Supervisor, run_with_restart_for_testing};
use suprnova::{Exceptions, FrameworkError, InertiaRequestExt, InertiaResponse};
use tokio_util::sync::CancellationToken;

struct Reports(Arc<Mutex<Vec<String>>>);

impl Reports {
    fn capture() -> Self {
        Exceptions::reset();
        let messages = Arc::new(Mutex::new(Vec::new()));
        let recorded = messages.clone();
        Exceptions::reportable(move |error: &FrameworkError| {
            recorded
                .lock()
                .expect("reports lock")
                .push(error.to_string());
        });
        Self(messages)
    }

    fn contains(&self, message: &str) {
        let reports = self.0.lock().expect("reports lock").clone();
        assert!(
            reports.iter().any(|report| report.contains(message)),
            "missing {message:?} in {reports:?}"
        );
    }
}

impl Drop for Reports {
    fn drop(&mut self) {
        Exceptions::reset();
    }
}

#[tokio::test]
#[serial]
async fn report_002_scheduled_failures_report_without_a_daemon() {
    let reports = Reports::capture();
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .call(|| async { Err(FrameworkError::internal("scheduled inline failure")) })
            .every_minute()
            .name("inline"),
    );
    schedule.add(
        schedule
            .call(|| async { Err(FrameworkError::internal("scheduled background failure")) })
            .every_minute()
            .name("background")
            .run_in_background(),
    );
    let results = schedule.run_all_tasks().await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|(_, outcome)| outcome.is_err()));
    reports.contains("scheduled inline failure");
    reports.contains("scheduled background failure");
}

#[tokio::test]
#[serial]
async fn report_002_scheduled_panics_keep_the_payload() {
    let reports = Reports::capture();
    let mut schedule = Schedule::new();
    schedule.add(
        schedule
            .call(|| async { panic!("scheduled inline panic payload") })
            .every_minute()
            .name("inline-panic"),
    );
    schedule.add(
        schedule
            .call(|| async { panic!("scheduled background panic payload") })
            .every_minute()
            .name("background-panic")
            .run_in_background(),
    );
    let results = schedule.run_all_tasks().await;
    assert_eq!(results.len(), 2);
    reports.contains("scheduled inline panic payload");
    reports.contains("scheduled background panic payload");
    assert!(
        results
            .iter()
            .any(|(_, result)| result.as_ref().is_err_and(|error| error
                .to_string()
                .contains("scheduled background panic payload")))
    );
}

struct FailingSupervisor {
    panic: bool,
}

#[async_trait]
impl Supervisor for FailingSupervisor {
    fn name(&self) -> &'static str {
        "reporting-one-shot"
    }
    async fn run(&self, _: CancellationToken) -> Result<(), FrameworkError> {
        if self.panic {
            panic!("supervisor panic payload")
        }
        Err(FrameworkError::internal("supervisor returned failure"))
    }
    fn restart_policy(&self) -> RestartPolicy {
        RestartPolicy::Never
    }
}

#[tokio::test]
#[serial]
async fn report_002_one_shot_supervisor_errors_are_reported() {
    let reports = Reports::capture();
    run_with_restart_for_testing(Arc::new(FailingSupervisor { panic: false })).await;
    reports.contains("supervisor returned failure");
}

#[tokio::test]
#[serial]
async fn report_002_one_shot_supervisor_panics_keep_the_payload() {
    let reports = Reports::capture();
    run_with_restart_for_testing(Arc::new(FailingSupervisor { panic: true })).await;
    reports.contains("supervisor panic payload");
}

struct PartialRequest;
impl InertiaRequestExt for PartialRequest {
    fn path(&self) -> &str {
        "/reporting"
    }
    fn path_and_query(&self) -> String {
        "/reporting".into()
    }
    fn header(&self, name: &str) -> Option<&str> {
        match name {
            "X-Inertia" => Some("true"),
            "X-Inertia-Partial-Component" => Some("Reporting"),
            "X-Inertia-Partial-Data" => Some("failed"),
            _ => None,
        }
    }
}

#[tokio::test]
#[serial]
async fn report_002_rescued_inertia_props_are_reported() {
    let reports = Reports::capture();
    let response = InertiaResponse::new("Reporting")
        .defer_with("failed", suprnova::DeferOptions::new().rescue(), || async {
            Err::<serde_json::Value, _>(FrameworkError::internal("rescued prop failure"))
        })
        .resolve(&PartialRequest)
        .await
        .expect("rescue keeps response successful");
    assert_eq!(response.status_code(), 200);
    reports.contains("rescued prop failure");
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PanicJob;

#[async_trait]
impl suprnova::Job for PanicJob {
    fn job_name() -> &'static str {
        "reporting::PanicJob"
    }
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        panic!("queued panic location payload")
    }
}

#[derive(Default)]
struct EventReports(Mutex<Vec<suprnova::ReportingEvent>>);
impl suprnova::Reporter for EventReports {
    fn report(&self, event: suprnova::ReportingEvent) -> Result<(), FrameworkError> {
        self.0.lock().expect("event reports lock").push(event);
        Ok(())
    }
    fn flush(&self, _: std::time::Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

#[tokio::test]
#[serial]
async fn report_006_http_transaction_records_a_bare_server_error_status() {
    let _container = suprnova::container::testing::TestContainer::fake();
    Exceptions::reset();
    let events = Arc::new(EventReports::default());
    suprnova::App::instance(suprnova::ReportingConfig::default().traces_sample_rate(1.0));
    suprnova::App::bind::<dyn suprnova::Reporter>(events.clone());
    suprnova::Reporting::bootstrap().expect("reporting boot");
    let client = suprnova::testing::TestClient::new(
        suprnova::Router::new().get("/unavailable", |_: suprnova::Request| async {
            Ok(suprnova::HttpResponse::text("unavailable").status(503))
        }),
        suprnova::MiddlewareRegistry::new(),
    );
    client.get("/unavailable").send().await.assert_status(503);
    let events = events.0.lock().expect("event reports lock").clone();
    let transaction = events
        .iter()
        .find(|event| event.kind == suprnova::ReportingKind::Transaction)
        .expect("sampled HTTP transaction");
    assert_eq!(transaction.payload["transaction"], "/unavailable");
    assert_eq!(
        transaction.payload["contexts"]["trace"]["status"], "internal_error",
        "response status determines the transaction result without an exception event"
    );
}

#[tokio::test]
#[serial]
async fn report_002_queue_panic_keeps_its_type_and_original_location() {
    use std::time::Duration;
    use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
    use suprnova::queue::{MemoryFailedJobStore, MemoryQueueDriver, Queue};
    Exceptions::reset();
    let events = Arc::new(EventReports::default());
    suprnova::App::bind::<dyn suprnova::Reporter>(events.clone());
    suprnova::Reporting::bootstrap().expect("reporting boot");
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));
    register_job::<PanicJob>();
    Queue::push(PanicJob).await.expect("push panic job");
    tokio::time::timeout(
        Duration::from_secs(5),
        run_worker(
            driver,
            WorkerConfig {
                visibility_timeout: Duration::from_secs(10),
                poll_interval: Duration::from_millis(1),
                max_jobs: Some(1),
                queues: Vec::new(),
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("worker deadline")
    .expect("worker settles");
    let events = events.0.lock().expect("event reports lock").clone();
    let panic_events: Vec<_> = events
        .iter()
        .filter(|event| {
            event
                .payload
                .to_string()
                .contains("job panicked: queued panic location payload")
        })
        .collect();
    assert_eq!(
        panic_events.len(),
        1,
        "one event after the panic attempt settles"
    );
    let exception = &panic_events[0].payload["exception"]["values"][0];
    assert_eq!(
        exception["type"], "panic",
        "caught job panic must retain its type"
    );
    assert!(
        exception["stacktrace"].to_string().contains("paths.rs"),
        "panic must retain its original source location: {exception}"
    );
}

#[suprnova::command(
    name = "reporting:boot-probe",
    description = "Checks reporting during bootstrap"
)]
async fn boot_probe(_args: Vec<String>) -> Result<(), FrameworkError> {
    Ok(())
}

#[tokio::test]
#[serial]
async fn report_002_bootstrap_panic_retains_original_location_once() {
    Exceptions::reset();
    let events = Arc::new(EventReports::default());
    suprnova::App::bind::<dyn suprnova::Reporter>(events.clone());
    let result = suprnova::console::dispatch_argv_with_init(
        vec!["console".into(), "reporting:boot-probe".into()],
        || async {
            panic!("bootstrap location payload");
        },
    )
    .await;
    assert!(
        result.is_err(),
        "bootstrap panic prevents command execution"
    );
    let events = events.0.lock().expect("event reports lock").clone();
    assert_eq!(
        events.len(),
        1,
        "bootstrap panic is reported once after winding down: {events:?}"
    );
    let exception = &events[0].payload["exception"]["values"][0];
    assert_eq!(
        exception["type"], "panic",
        "bootstrap panic retains its type"
    );
    assert!(
        exception["stacktrace"].to_string().contains("paths.rs"),
        "bootstrap panic retains its location: {exception}"
    );
}
