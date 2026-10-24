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

/// Issue #158: a panic the hook reports from inside a scope carries the
/// scope's request id, as an error reported in that scope does.
#[test]
#[serial]
fn report_002_panic_inside_a_scope_carries_its_request_id() {
    Exceptions::reset();
    Reporting::configure(ReportingConfig::default().driver(ReportingDriver::Null))
        .expect("configure");
    let sink = Arc::new(Capture::default());
    App::bind::<dyn Reporter>(sink.clone());
    Reporting::bootstrap().expect("bootstrap custom reporter");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let store = suprnova::ContextStore::with_query(Default::default());
    let caught = runtime.block_on(suprnova::Context::scope(store, async {
        suprnova::Context::add("_request_id", "req-158");
        std::panic::catch_unwind(|| panic!("scoped panic payload"))
    }));
    assert!(caught.is_err());
    assert!(Reporting::flush(Duration::from_secs(2)).expect("flush hook and sender"));
    let events = sink.0.lock().expect("capture");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["tags"]["request_id"], "req-158");
}
