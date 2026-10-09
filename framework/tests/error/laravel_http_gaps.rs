//! The tenth parity round's error blocks: `Exceptions` reporting
//! (PAR-111) and the development error page's type name and editor links
//! (PAR-112). Each test is named after the falsifier clause it observes.
//!
//! # Isolation
//!
//! The `Exceptions` registry is process-wide. Every test here runs
//! `#[serial]`, holds the binary's env lock for its whole body
//! ([`page_mode`]), and empties the registry at its start and end
//! ([`Isolated`]). Its callbacks match error types or messages no other
//! test uses, so a 5xx another test of this binary produces at the same
//! time under plain `cargo test` reaches them without changing what they
//! record.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serial_test::serial;
use suprnova::config::Config;
use suprnova::{Exceptions, FrameworkError, HttpResponse, Request, Response, Router, command};

use crate::debug_error_page::{BROWSER, JSON_CLIENT, Reply, assert_debug_page, get};
use crate::env_snapshot::{EnvSnapshot, set_env};

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

/// Holds the env lock with `APP_DEBUG` and `APP_EDITOR` set, and restores
/// both when dropped. The snapshot is declared first, so it is restored
/// before the lock is released.
struct PageMode {
    _env: EnvSnapshot,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Debug mode `debug`, and `APP_EDITOR` set to `editor` or removed.
async fn page_mode(debug: bool, editor: Option<&str>) -> PageMode {
    let lock = crate::env_lock::lock_env_async().await;
    let snapshot = EnvSnapshot::capture(&["APP_DEBUG", "APP_EDITOR"]);
    set_env("APP_DEBUG", Some(if debug { "true" } else { "false" }));
    set_env("APP_EDITOR", editor);
    assert_eq!(
        Config::is_debug(),
        debug,
        "Config::is_debug() must read the APP_DEBUG this test set"
    );
    PageMode {
        _env: snapshot,
        _lock: lock,
    }
}

/// Empties the `Exceptions` registry when made and when dropped.
struct Isolated;

impl Isolated {
    fn new() -> Self {
        Exceptions::reset();
        Self
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        Exceptions::reset();
    }
}

/// What a callback received, one entry per call.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<String>>>);

impl Seen {
    fn push(&self, text: String) {
        self.0.lock().expect("the list is not poisoned").push(text);
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().expect("the list is not poisoned").clone()
    }

    fn contains(&self, text: &str) -> bool {
        self.all().iter().any(|seen| seen.contains(text))
    }
}

/// The log lines written on this thread while the guard is held. Every
/// test here runs on a current-thread runtime, so every task of the
/// request writes on this thread.
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        (logs, tracing::subscriber::set_default(subscriber))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("the buffer is not poisoned")).into_owned()
    }

    /// Whether a `framework error` line names `message`.
    fn has_framework_error_line(&self, message: &str) -> bool {
        self.text()
            .lines()
            .any(|line| line.contains("framework error") && line.contains(message))
    }
}

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("the buffer is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// An error type only this file uses, so a callback typed on it receives
/// no other test's errors.
#[derive(Debug)]
struct LedgerClosed(&'static str);

impl std::fmt::Display for LedgerClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the ledger is closed: {}", self.0)
    }
}

impl std::error::Error for LedgerClosed {}

/// An error whose own source is an `std::io::Error`.
#[derive(Debug)]
struct ArchiveFailed {
    cause: std::io::Error,
}

impl std::fmt::Display for ArchiveFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("archiving the ledger failed")
    }
}

impl std::error::Error for ArchiveFailed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// A route that fails with `error` built fresh on each request.
fn failing(path: &'static str, error: fn() -> FrameworkError) -> Router {
    Router::new()
        .get(path, move |_req: Request| async move {
            Err::<HttpResponse, _>(HttpResponse::from(error()))
        })
        .into()
}

// ---------------------------------------------------------------------
// PAR-111: Exceptions::reportable, report, dont_retry
// ---------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_callback_taking_an_io_error_runs_when_from_external_of_one_becomes_a_500() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let seen = Seen::default();
    let record = seen.clone();
    Exceptions::reportable(move |error: &std::io::Error| record.push(error.to_string()));

    let reply = get(
        failing("/disk", || {
            FrameworkError::from_external(std::io::Error::other("ledger volume 9 unplugged"))
        }),
        "/disk",
        JSON_CLIENT,
    )
    .await;

    assert_eq!(reply.status, 500, "{}", reply.body);
    assert!(
        seen.contains("ledger volume 9 unplugged"),
        "the io callback must receive the wrapped io error; it saw {:?}",
        seen.all()
    );
}

#[tokio::test]
#[serial]
async fn a_callback_taking_an_io_error_does_not_run_for_an_internal_error() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let io_seen = Seen::default();
    let record_io = io_seen.clone();
    Exceptions::reportable(move |error: &std::io::Error| record_io.push(error.to_string()));
    let all_seen = Seen::default();
    let record_all = all_seen.clone();
    Exceptions::reportable(move |error: &FrameworkError| record_all.push(error.to_string()));

    let reply = get(
        failing("/internal", || {
            FrameworkError::internal("ledger x-ray 41 overflowed")
        }),
        "/internal",
        JSON_CLIENT,
    )
    .await;

    assert_eq!(reply.status, 500, "{}", reply.body);
    assert!(
        all_seen.contains("ledger x-ray 41 overflowed"),
        "a FrameworkError callback receives every reported error; it saw {:?}",
        all_seen.all()
    );
    assert!(
        !io_seen.contains("ledger x-ray 41 overflowed"),
        "an internal error wraps no io error; the io callback saw {:?}",
        io_seen.all()
    );
}

#[tokio::test]
#[serial]
async fn a_callback_compares_the_wrapped_source_and_not_the_sources_own_chain() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let io_seen = Seen::default();
    let record_io = io_seen.clone();
    Exceptions::reportable(move |error: &std::io::Error| record_io.push(error.to_string()));
    let archive_seen = Seen::default();
    let record_archive = archive_seen.clone();
    Exceptions::reportable(move |error: &ArchiveFailed| record_archive.push(error.to_string()));

    Exceptions::report(&FrameworkError::from_external(ArchiveFailed {
        cause: std::io::Error::other("archive volume 4 is read-only"),
    }));

    assert_eq!(archive_seen.all(), ["archiving the ledger failed"]);
    assert!(
        !io_seen.contains("archive volume 4 is read-only"),
        "the io error is the wrapped source's own source, which is not compared; \
         the io callback saw {:?}",
        io_seen.all()
    );
}

#[tokio::test]
#[serial]
async fn a_stopping_callback_keeps_the_log_line_and_later_callbacks_from_running() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let (logs, _subscriber) = Logs::capture();
    let stopper_seen = Seen::default();
    let record_stopper = stopper_seen.clone();
    Exceptions::reportable(move |error: &LedgerClosed| record_stopper.push(error.to_string()))
        .stop();
    let later_seen = Seen::default();
    let record_later = later_seen.clone();
    Exceptions::reportable(move |error: &FrameworkError| record_later.push(error.to_string()));

    let stopped = get(
        failing("/closed", || {
            FrameworkError::from_external(LedgerClosed("night batch 77"))
        }),
        "/closed",
        JSON_CLIENT,
    )
    .await;
    let reported = get(
        failing("/open", || {
            FrameworkError::internal("ledger y-ray 52 overflowed")
        }),
        "/open",
        JSON_CLIENT,
    )
    .await;

    assert_eq!(stopped.status, 500, "{}", stopped.body);
    assert_eq!(reported.status, 500, "{}", reported.body);
    assert!(stopper_seen.contains("night batch 77"));
    assert!(
        !later_seen.contains("night batch 77"),
        "a callback after a stopping one must not run for the error it took; it saw {:?}",
        later_seen.all()
    );
    assert!(
        !logs.has_framework_error_line("night batch 77"),
        "the default log line must not run for an error a stopping callback took:\n{}",
        logs.text()
    );
    assert!(
        later_seen.contains("ledger y-ray 52 overflowed"),
        "an error the stopping callback does not take is reported on; the later callback \
         saw {:?}",
        later_seen.all()
    );
    assert!(
        logs.has_framework_error_line("ledger y-ray 52 overflowed"),
        "and logged:\n{}",
        logs.text()
    );
}

#[tokio::test]
#[serial]
async fn report_runs_the_callbacks_in_order_then_writes_the_log_line() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let (logs, _subscriber) = Logs::capture();
    let seen = Seen::default();
    let first = seen.clone();
    Exceptions::reportable(move |error: &FrameworkError| first.push(format!("first: {error}")));
    let second = seen.clone();
    Exceptions::reportable(move |error: &LedgerClosed| second.push(format!("second: {error}")));

    Exceptions::report(&FrameworkError::from_external(LedgerClosed("handled 3")));

    // Filtered: under plain `cargo test` a 5xx another test makes also
    // reaches the first callback.
    let mine: Vec<String> = seen
        .all()
        .into_iter()
        .filter(|entry| entry.contains("handled 3"))
        .collect();
    assert_eq!(
        mine,
        [
            "first: the ledger is closed: handled 3",
            "second: the ledger is closed: handled 3"
        ]
    );
    assert!(
        logs.has_framework_error_line("the ledger is closed: handled 3"),
        "{}",
        logs.text()
    );
}

#[tokio::test]
#[serial]
async fn a_panicking_callback_is_logged_and_reporting_goes_on() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let (logs, _subscriber) = Logs::capture();
    Exceptions::reportable(|_: &LedgerClosed| panic!("the alert hook is down"));
    let seen = Seen::default();
    let record = seen.clone();
    Exceptions::reportable(move |error: &FrameworkError| record.push(error.to_string()));

    Exceptions::report(&FrameworkError::from_external(LedgerClosed("hook 5")));

    assert!(seen.contains("hook 5"), "{:?}", seen.all());
    assert!(
        logs.text().contains("the alert hook is down"),
        "{}",
        logs.text()
    );
    assert!(logs.has_framework_error_line("hook 5"), "{}", logs.text());
}

/// Fails with an error only this file reports.
#[command(
    name = "laravel-http-gaps:close-ledger",
    description = "Fails with LedgerClosed"
)]
async fn close_ledger_command(_args: Vec<String>) -> Result<(), FrameworkError> {
    Err(FrameworkError::from_external(LedgerClosed("console run 8")))
}

/// Fails with the sentinel that says the user has seen the failure.
#[command(
    name = "laravel-http-gaps:already-reported",
    description = "Fails silently"
)]
async fn already_reported_command(_args: Vec<String>) -> Result<(), FrameworkError> {
    Err(FrameworkError::silent())
}

#[tokio::test]
#[serial]
async fn a_console_command_that_returns_err_reaches_the_callbacks() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let seen = Seen::default();
    let record = seen.clone();
    Exceptions::reportable(move |error: &LedgerClosed| record.push(error.to_string()));
    let silent = Seen::default();
    let record_silent = silent.clone();
    Exceptions::reportable(move |error: &FrameworkError| {
        if error.is_silent() {
            record_silent.push("silent".to_string());
        }
    });

    let failed = suprnova::console::dispatch_argv(vec![
        "console".to_string(),
        "laravel-http-gaps:close-ledger".to_string(),
    ])
    .await;
    let quiet = suprnova::console::dispatch_argv(vec![
        "console".to_string(),
        "laravel-http-gaps:already-reported".to_string(),
    ])
    .await;

    assert!(failed.is_err(), "the command's error is returned");
    assert_eq!(seen.all(), ["the ledger is closed: console run 8"]);
    assert!(quiet.expect_err("the sentinel is returned").is_silent());
    assert!(
        silent.all().is_empty(),
        "AlreadyReported says the user has seen it; it is not reported"
    );
}

#[tokio::test]
#[serial]
async fn should_stop_retries_answers_for_dont_retry_and_dont_retry_when() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    let closed = FrameworkError::from_external(LedgerClosed("retry 1"));
    let declined = FrameworkError::domain("card declined", 402);
    let other = FrameworkError::internal("ledger z-ray 63 overflowed");
    assert!(!Exceptions::should_stop_retries(&closed));
    assert!(!Exceptions::should_stop_retries(&declined));

    Exceptions::dont_retry::<LedgerClosed>();
    Exceptions::dont_retry_when(|error: &FrameworkError| error.status_code() == 402);

    assert!(Exceptions::should_stop_retries(&closed));
    assert!(Exceptions::should_stop_retries(&declined));
    assert!(!Exceptions::should_stop_retries(&other));
}

#[tokio::test]
#[serial]
async fn a_panicking_dont_retry_when_predicate_counts_as_false() {
    let _mode = page_mode(false, None).await;
    let _isolated = Isolated::new();
    Exceptions::dont_retry_when(|_: &FrameworkError| panic!("the retry policy store is down"));

    assert!(!Exceptions::should_stop_retries(&FrameworkError::internal(
        "ledger w-ray 70 overflowed"
    )));
}

// ---------------------------------------------------------------------
// PAR-112: the page names the error's type and links frames to an editor
// ---------------------------------------------------------------------

/// The line [`fail_on_ledger_disk`] creates its error on, stored when it
/// runs, so the check does not depend on how this file is formatted.
static DISK_LINE: AtomicU32 = AtomicU32::new(0);

/// Fails with a wrapped io error, created on a line the tests read back.
fn fail_on_ledger_disk() -> Result<(), FrameworkError> {
    DISK_LINE.store(line!() + 1, Ordering::SeqCst);
    Err(FrameworkError::from_external(std::io::Error::other("disk")))
}

async fn write_ledger(_req: Request) -> Response {
    fail_on_ledger_disk()?;
    HttpResponse::text("written").ok()
}

fn disk_routes() -> Router {
    Router::new().get("/ledger-disk", write_ledger).into()
}

/// The absolute path of this file, as the page must link it: the
/// workspace root joined with the path `file!()` gives.
fn this_file() -> String {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap_or(manifest)
        .join(file!())
        .to_string_lossy()
        .into_owned()
}

/// The `<li class="app">` element of the shown frame named `function`.
fn app_frame<'a>(reply: &'a Reply, function: &str) -> &'a str {
    reply
        .body
        .split("<li class=\"app\">")
        .skip(1)
        .find(|frame| {
            frame
                .split("</code>")
                .next()
                .is_some_and(|name| name.contains(function))
        })
        .and_then(|frame| frame.split("</li>").next())
        .unwrap_or_else(|| {
            panic!(
                "the page must show the application frame `{function}`; page:\n{}",
                reply.body
            )
        })
}

/// The `<p class="type">` text of the page.
fn type_line(reply: &Reply) -> String {
    let text = reply.text();
    text.split("<p class=\"type\"><code>")
        .nth(1)
        .and_then(|rest| rest.split("</code>").next())
        .unwrap_or_else(|| panic!("the page must name the type; page:\n{}", reply.body))
        .to_string()
}

#[tokio::test]
#[serial]
async fn the_page_for_a_500_from_an_external_io_error_names_the_io_error_type() {
    let _mode = page_mode(true, None).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    assert_eq!(
        type_line(&reply),
        std::any::type_name::<std::io::Error>(),
        "page:\n{}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn a_panics_page_names_panic() {
    let _mode = page_mode(true, None).await;

    let reply = get(
        crate::debug_error_page::ledger_routes(),
        "/ledger-index",
        BROWSER,
    )
    .await;

    assert_debug_page(&reply, 500);
    assert_eq!(type_line(&reply), "panic", "page:\n{}", reply.body);
}

/// An error type no `from_external` call wraps, for a struct literal.
#[derive(Debug)]
struct NeverWrapped;

impl std::fmt::Display for NeverWrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("built as a struct literal")
    }
}

impl std::error::Error for NeverWrapped {}

#[test]
#[serial]
fn type_name_names_the_wrapped_type_or_the_framework_variant() {
    let type_name = |error: FrameworkError| {
        HttpResponse::from(error)
            .error_report()
            .expect("a 4xx or 5xx from an error carries a report")
            .type_name()
            .map(str::to_string)
    };

    assert_eq!(
        type_name(FrameworkError::from_external(std::io::Error::other("disk"))).as_deref(),
        Some(std::any::type_name::<std::io::Error>())
    );
    assert_eq!(
        type_name(FrameworkError::from_external_with(
            "saving the ledger",
            LedgerClosed("type 2")
        ))
        .as_deref(),
        Some(std::any::type_name::<LedgerClosed>())
    );
    assert_eq!(
        type_name(FrameworkError::model_not_found("Invoice")).as_deref(),
        Some("FrameworkError::ModelNotFound")
    );
    assert_eq!(
        type_name(FrameworkError::internal("x")).as_deref(),
        Some("FrameworkError::Internal")
    );
    assert_eq!(
        type_name(
            FrameworkError::from_external_with("saving", std::io::Error::other("disk"))
                .context("posting")
        )
        .as_deref(),
        Some(std::any::type_name::<std::io::Error>()),
        "context keeps the wrapped source, so it keeps the name"
    );
    assert_eq!(
        type_name(FrameworkError::External {
            message: "built by hand".into(),
            source: Arc::new(NeverWrapped),
        })
        .as_deref(),
        Some("FrameworkError::External"),
        "a struct literal around a type no constructor wrapped falls back to the variant"
    );
}

#[tokio::test]
#[serial]
async fn with_app_editor_vscode_an_application_frame_links_to_its_absolute_file_and_line() {
    let _mode = page_mode(true, Some("vscode")).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    let line = DISK_LINE.load(Ordering::SeqCst);
    let frame = app_frame(&reply, "fail_on_ledger_disk");
    let href = format!("href=\"vscode://file/{}:{line}\"", this_file());
    assert!(
        frame.contains(&href),
        "the frame must carry {href}; frame:\n{frame}"
    );
    assert!(
        frame.contains("<pre class=\"source\">"),
        "the source lines stay beside the link; frame:\n{frame}"
    );
}

#[tokio::test]
#[serial]
async fn a_known_editor_name_uses_that_editors_url_format() {
    let _mode = page_mode(true, Some("phpstorm")).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    let line = DISK_LINE.load(Ordering::SeqCst);
    let frame = app_frame(&reply, "fail_on_ledger_disk");
    let href = format!(
        "href=\"phpstorm://open?file={}&amp;line={line}\"",
        this_file()
    );
    assert!(
        frame.contains(&href),
        "the frame must carry {href}, escaped as page text is; frame:\n{frame}"
    );
}

#[tokio::test]
#[serial]
async fn an_app_editor_template_is_filled_with_the_file_and_line() {
    let _mode = page_mode(true, Some("myeditor://{file}#{line}")).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    let line = DISK_LINE.load(Ordering::SeqCst);
    let frame = app_frame(&reply, "fail_on_ledger_disk");
    let href = format!("href=\"myeditor://{}#{line}\"", this_file());
    assert!(
        frame.contains(&href),
        "the frame must carry {href}; frame:\n{frame}"
    );
}

#[tokio::test]
#[serial]
async fn another_app_editor_name_opens_through_its_own_scheme() {
    let _mode = page_mode(true, Some("myeditor")).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    let line = DISK_LINE.load(Ordering::SeqCst);
    let frame = app_frame(&reply, "fail_on_ledger_disk");
    let href = format!(
        "href=\"myeditor://open?file={}&amp;line={line}\"",
        this_file()
    );
    assert!(
        frame.contains(&href),
        "the frame must carry {href}; frame:\n{frame}"
    );
}

#[tokio::test]
#[serial]
async fn without_app_editor_no_frame_carries_a_link() {
    let _mode = page_mode(true, None).await;

    let reply = get(disk_routes(), "/ledger-disk", BROWSER).await;

    assert_debug_page(&reply, 500);
    let frame = app_frame(&reply, "fail_on_ledger_disk");
    assert!(
        frame.contains("<span class=\"at\">"),
        "the frame still shows its location; frame:\n{frame}"
    );
    assert!(
        !reply.body.contains("<a "),
        "with APP_EDITOR unset no frame links anywhere; page:\n{}",
        reply.body
    );
}
