use super::drivers::Capture;
use serial_test::serial;
use std::sync::Arc;
use suprnova::{
    App, AppError, Exceptions, FrameworkError, Reporter, report, report_if, report_unless, rescue,
};

#[tokio::test]
#[serial]
async fn report_002_helpers_report_only_selected_failures_and_rescue_returns_fallback() {
    Exceptions::reset();
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    report_if(false, &FrameworkError::internal("skipped if")).expect("report_if");
    report_unless(true, &FrameworkError::internal("skipped unless")).expect("report_unless");
    report(&FrameworkError::internal("explicit")).expect("report");
    report_if(true, &FrameworkError::internal("if")).expect("report_if");
    report_unless(false, &FrameworkError::internal("unless")).expect("report_unless");
    let value = rescue(
        async { Err::<usize, _>(FrameworkError::internal("rescued")) },
        |_| 42,
    )
    .await;
    assert_eq!(value, 42);
    assert_eq!(sink.0.lock().expect("capture").len(), 4);
}

#[test]
#[serial]
fn report_002_expected_http_refusals_and_silent_errors_are_excluded() {
    Exceptions::reset();
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    for code in [400, 401, 403, 404, 419, 422, 429] {
        Exceptions::report(&FrameworkError::from(
            AppError::new("expected refusal").status(code),
        ));
    }
    Exceptions::report(&FrameworkError::AlreadyReported);
    assert!(sink.0.lock().expect("capture").is_empty());
}

#[test]
#[serial]
fn report_002_suppression_predicates_types_and_duplicate_instances_are_honored() {
    Exceptions::reset();
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    Exceptions::dont_report::<std::io::Error>();
    Exceptions::dont_report_when(|error| error.to_string().contains("ignore this"));
    Exceptions::dont_report_duplicates();
    Exceptions::report(&FrameworkError::from_external(std::io::Error::other(
        "disk",
    )));
    Exceptions::report(&FrameworkError::internal("ignore this"));
    let error = FrameworkError::internal("unique incident");
    Exceptions::report(&error);
    Exceptions::report(&error);
    assert_eq!(sink.0.lock().expect("capture").len(), 1);
}
