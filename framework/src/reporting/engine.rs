use std::cell::Cell;
use std::future::Future;

tokio::task_local! { static REPORTED: Cell<bool>; }

pub(crate) async fn capture_engine_reports<F: Future>(future: F) -> (F::Output, bool) {
    REPORTED
        .scope(Cell::new(false), async {
            let result = future.await;
            (result, REPORTED.with(Cell::get))
        })
        .await
}

pub(crate) fn report_engine_panic(message: &str, kind: suprnova_live::PanicKind) {
    if kind == suprnova_live::PanicKind::Operation {
        let _ = REPORTED.try_with(|reported| reported.set(true));
    }
    // Outside a catch boundary the global hook already queued this panic.
    if crate::error::in_panic_boundary() {
        let location = crate::error::recorded_panic_location(message);
        crate::Exceptions::report_panic(message, location.as_deref());
    }
}

#[derive(Debug)]
struct ReportedEngineError(String);
impl std::fmt::Display for ReportedEngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for ReportedEngineError {}

pub(crate) fn reported_engine_error(message: impl Into<String>) -> crate::FrameworkError {
    crate::FrameworkError::from_external(ReportedEngineError(message.into()))
}

pub(crate) fn already_reported_engine_error(error: &crate::FrameworkError) -> bool {
    error
        .external_source()
        .is_some_and(|source| source.is::<ReportedEngineError>())
}
