use serial_test::serial;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::{
    App, Exceptions, FrameworkError, Reporter, Reporting, ReportingConfig, ReportingDriver,
    ReportingEvent,
};

#[derive(Default)]
pub(crate) struct Capture(pub(crate) Mutex<Vec<ReportingEvent>>);
impl Reporter for Capture {
    fn report(&self, event: ReportingEvent) -> Result<(), FrameworkError> {
        self.0.lock().expect("capture lock").push(event);
        Ok(())
    }
    fn flush(&self, _timeout: Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

#[test]
#[serial]
fn report_001_exceptions_reach_the_bound_reporter_after_callbacks() {
    Exceptions::reset();
    let capture = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(capture.clone());
    Exceptions::reportable(|_: &FrameworkError| {});
    Exceptions::report(&FrameworkError::internal("ledger unavailable"));
    let events = capture.0.lock().expect("capture lock");
    assert_eq!(events.len(), 1);
    assert!(events[0].payload.to_string().contains("ledger unavailable"));
}

#[test]
#[serial]
fn report_001_stopping_callback_prevents_delivery() {
    Exceptions::reset();
    let capture = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(capture.clone());
    Exceptions::reportable(|_: &FrameworkError| {}).stop();
    Exceptions::report(&FrameworkError::internal("stopped"));
    assert!(capture.0.lock().expect("capture lock").is_empty());
}

#[test]
#[serial]
fn report_001_null_and_sentry_without_dsn_are_inert() {
    for driver in [ReportingDriver::Null, ReportingDriver::Sentry] {
        let reporter = ReportingConfig::default()
            .driver(driver)
            .build()
            .expect("build reporter");
        reporter
            .report(ReportingEvent::from_error(&FrameworkError::internal(
                "no network",
            )))
            .expect("report");
        assert!(reporter.flush(Duration::from_millis(100)).expect("flush"));
    }
}

#[test]
#[serial]
fn report_001_unknown_driver_fails_at_the_configuration_boundary() {
    assert!(ReportingDriver::parse("not-a-driver").is_err());
    assert!(Reporting::configure(ReportingConfig::default().driver(ReportingDriver::Log)).is_ok());
}

struct PanickingReporter;
impl Reporter for PanickingReporter {
    fn report(&self, _: ReportingEvent) -> Result<(), FrameworkError> {
        panic!("custom reporter bug");
    }
    fn flush(&self, _: Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

#[tokio::test]
#[serial]
async fn report_005_a_panicking_custom_driver_cannot_fail_the_request() {
    Exceptions::reset();
    App::bind::<dyn Reporter>(Arc::new(PanickingReporter));
    let client = suprnova::testing::TestClient::new(
        suprnova::Router::new().get("/failure", |_: suprnova::Request| async {
            Err::<suprnova::HttpResponse, _>(suprnova::HttpResponse::from(
                FrameworkError::internal("original failure"),
            ))
        }),
        suprnova::MiddlewareRegistry::new(),
    );
    client.get("/failure").send().await.assert_status(500);
}

#[test]
#[serial]
fn report_005_a_panicking_driver_returns_an_error_to_explicit_report() {
    Exceptions::reset();
    App::bind::<dyn Reporter>(Arc::new(PanickingReporter));
    let result = std::panic::catch_unwind(|| {
        suprnova::report(&FrameworkError::internal("explicit failure"))
    });
    assert!(
        matches!(result, Ok(Err(_))),
        "a broken driver must become a Result error"
    );
}
