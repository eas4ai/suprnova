use super::drivers::Capture;
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::{
    App, Exceptions, FrameworkError, Reporter, Reporting, ReportingConfig, ReportingDriver,
};

#[test]
#[serial]
fn report_002_panic_outside_a_request_keeps_payload_and_location() {
    Exceptions::reset();
    Reporting::configure(ReportingConfig::default().driver(ReportingDriver::Null))
        .expect("configure");
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    Reporting::bootstrap().expect("bootstrap custom reporter");
    assert!(
        std::thread::spawn(|| panic!("off-request panic payload"))
            .join()
            .is_err()
    );
    assert!(Reporting::flush(Duration::from_secs(2)).expect("flush hook and sender"));
    let events = sink.0.lock().expect("capture");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["exception"]["values"][0]["type"], "panic");
    assert!(events[0].payload.to_string().contains("hook.rs"));
}

#[test]
#[serial]
fn report_002_panicking_callback_cannot_abort_the_panic_hook_or_recurse() {
    Exceptions::reset();
    Reporting::configure(ReportingConfig::default().driver(ReportingDriver::Null))
        .expect("configure");
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    Reporting::bootstrap().expect("bootstrap");
    Exceptions::reportable(|_: &FrameworkError| panic!("broken application callback"));
    assert!(
        std::thread::spawn(|| panic!("original incident"))
            .join()
            .is_err()
    );
    assert!(Reporting::flush(Duration::from_secs(2)).expect("flush"));
    assert_eq!(sink.0.lock().expect("capture").len(), 1);
}
