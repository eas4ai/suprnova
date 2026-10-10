//! The logging rows of Laravel's infrastructure surface.
//!
//! The `Log` facade's level methods write to the default channel through
//! `tracing` and keep their PSR-3 level; loggers carry context; shared
//! context is scoped to the current `Context` scope; every write dispatches
//! `MessageLogged` and reaches `Log::listen` (PAR-138). The built-in file and
//! stream channels keep only the records at or above `LOG_LEVEL`'s bare
//! level, and a channel chooses whether `{key}` placeholders are replaced
//! (PAR-139).
//!
//! The tests of the built-in channels run in a child process with its own
//! `APP_BASE_PATH` and `LOG_FORMAT=pretty`, since the storage path is fixed
//! once per process and an unset `APP_ENV` is production, whose default
//! format is JSON.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use suprnova::context::{Context, ContextStore};
use suprnova::events::dispatched;
use suprnova::logging::{LogConfig, LogFormat, build_subscriber};
use suprnova::{
    EventFacade, FrameworkError, Listener, Log, LogChannel, LogLevel, LogRecord, LogSink,
    MessageLogged,
};

use crate::env_lock::lock_env;
use crate::env_snapshot::{EnvSnapshot, set_env};

/// The variables a child must not inherit from the parent's environment.
const VARIABLES: &[&str] = &[
    "LOG_CHANNEL",
    "LOG_CHANNEL_DRIVER",
    "LOG_STACK",
    "LOG_LEVEL",
    "LOG_FORMAT",
    "LOG_DAILY_DAYS",
    "LOG_SYSLOG_SOCKET",
    "LOG_SYSLOG_FACILITY",
    "APP_BASE_PATH",
    "APP_NAME",
    "APP_ENV",
];

/// A name no other test uses, so the process-wide registries never mix two
/// tests' channels, drivers or messages.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Every file under `dir`, concatenated, for a dated file name.
fn read_dir(dir: &Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        text.push_str(&read(&entry.path()));
    }
    text
}

/// A sink that keeps what it is given, for `Log::extend`.
#[derive(Default)]
struct MemorySink {
    records: Mutex<Vec<LogRecord>>,
}

impl LogSink for MemorySink {
    fn write(&self, record: &LogRecord) -> std::io::Result<()> {
        self.records.lock().unwrap().push(record.clone());
        Ok(())
    }
}

impl MemorySink {
    fn records(&self) -> Vec<LogRecord> {
        self.records.lock().unwrap().clone()
    }
}

/// A logger on a driver of its own, writing into the returned sink.
fn memory_logger() -> (suprnova::Logger, Arc<MemorySink>) {
    let sink = Arc::new(MemorySink::default());
    let shared = Arc::clone(&sink);
    let driver = unique("memory");
    Log::extend(&driver, move |_| Ok(shared.clone() as Arc<dyn LogSink>));
    let logger = Log::build(LogChannel::driver(&driver)).expect("the driver builds");
    (logger, sink)
}

fn context_value(record: &LogRecord, key: &str) -> Option<String> {
    record
        .context
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

// ---- The child process -------------------------------------------------------

const CHILD: &str = "SUPRNOVA_LOG_INFRA_CHILD";

fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Run the test `name` of this binary alone in a child process with `env`
/// set, `LOG_FORMAT=pretty` unless `env` sets it, and the variables above
/// unset otherwise; return its output.
fn run_child(name: &str, env: &[(&str, &str)]) -> std::process::Output {
    let _env = lock_env();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for variable in VARIABLES {
        command.env_remove(variable);
    }
    command.env("LOG_FORMAT", "pretty");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "the child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Install the subscriber the server installs, run `write`, and flush the
/// file channels with a clean shutdown.
fn as_the_server(write: impl FnOnce()) {
    let guard = suprnova::telemetry::init_telemetry(
        LogConfig::from_env(),
        suprnova::telemetry::OtelConfig::disabled(),
    );
    write();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(guard.shutdown());
}

/// In a child: every facade level, `Log::log`, and the `_with` forms.
#[test]
fn child_writes_each_facade_level() {
    if !is_child() {
        return;
    }
    as_the_server(|| {
        Log::emergency("m-emergency");
        Log::alert("m-alert");
        Log::critical("disk full");
        Log::error("m-error");
        Log::warning("m-warning");
        Log::notice("m-notice");
        Log::info("m-info");
        Log::debug("m-debug");
        Log::warning_with("x", json!({"id": 7}));
        Log::critical_with("disk {disk} full", json!({"disk": "sda"}));
        Log::log(LogLevel::Alert, "m-log", json!({"via": "log"}));
    });
}

#[test]
fn facade_levels_keep_their_own_level_in_a_file_channel() {
    let base = tempfile::tempdir().unwrap();
    run_child(
        "laravel_infra_gaps::child_writes_each_facade_level",
        &[
            ("LOG_CHANNEL", "single"),
            ("LOG_LEVEL", "debug"),
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
        ],
    );
    let text = read(&base.path().join("storage/logs/suprnova.log"));
    let line_of = |message: &str| {
        text.lines()
            .find(|line| line.contains(&format!(": {message}")))
            .unwrap_or_else(|| panic!("no line for {message}:\n{text}"))
            .to_owned()
    };
    for (message, level) in [
        ("m-emergency", "EMERGENCY"),
        ("m-alert", "ALERT"),
        ("disk full", "CRITICAL"),
        ("m-error", "ERROR"),
        ("m-warning", "WARNING"),
        ("m-notice", "NOTICE"),
        ("m-info", "INFO"),
        ("m-debug", "DEBUG"),
        ("m-log", "ALERT"),
    ] {
        let line = line_of(message);
        assert!(line.contains(&format!("] {level}: ")), "{line}");
    }
    let line = line_of("x");
    assert!(line.contains("] WARNING: "), "{line}");
    assert!(line.contains(r#""id":"7""#), "the context id of 7: {line}");
    let line = line_of("disk sda full");
    assert!(line.contains("] CRITICAL: "), "{line}");
    assert!(line_of("m-log").contains(r#""via":"log""#));
}

#[test]
fn facade_levels_keep_their_own_level_in_a_stack() {
    let base = tempfile::tempdir().unwrap();
    run_child(
        "laravel_infra_gaps::child_writes_each_facade_level",
        &[
            ("LOG_CHANNEL", "stack"),
            ("LOG_STACK", "single"),
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
        ],
    );
    let text = read(&base.path().join("storage/logs/suprnova.log"));
    assert!(text.contains("] CRITICAL: disk full"), "{text}");
    assert!(text.contains("] NOTICE: m-notice"), "{text}");
}

// ---- Logger context ----------------------------------------------------------

#[test]
fn with_context_carries_context_on_each_later_write() {
    let (logger, sink) = memory_logger();
    let tenant = logger.with_context(json!({"tenant": "acme", "region": "eu"}));

    tenant.info("first");
    tenant.warning_with("second", json!({"region": "us", "id": 7}));
    logger.info("untouched");

    let records = sink.records();
    assert_eq!(records.len(), 3);
    assert_eq!(
        context_value(&records[0], "tenant").as_deref(),
        Some("acme")
    );
    assert_eq!(
        context_value(&records[1], "tenant").as_deref(),
        Some("acme")
    );
    assert_eq!(
        context_value(&records[1], "region").as_deref(),
        Some("us"),
        "the call's own context wins"
    );
    assert_eq!(context_value(&records[1], "id").as_deref(), Some("7"));
    assert_eq!(
        context_value(&records[2], "tenant"),
        None,
        "the logger it came from is unchanged"
    );
}

#[test]
fn without_context_drops_the_keys_named_or_all() {
    let (logger, sink) = memory_logger();
    let tenant = logger.with_context(json!({"tenant": "acme", "region": "eu"}));

    tenant.without_context(Some(&["region"])).info("one");
    tenant.without_context(None).info("two");

    let records = sink.records();
    assert_eq!(
        context_value(&records[0], "tenant").as_deref(),
        Some("acme")
    );
    assert_eq!(context_value(&records[0], "region"), None);
    assert!(records[1].context.is_empty(), "{:?}", records[1].context);
}

#[test]
fn every_logger_level_has_a_with_form() {
    let (logger, sink) = memory_logger();
    let context = || json!({"k": "v"});
    logger.emergency_with("a", context());
    logger.alert_with("b", context());
    logger.critical_with("c", context());
    logger.error_with("d", context());
    logger.warning_with("e", context());
    logger.notice_with("f", context());
    logger.info_with("g", context());
    logger.debug_with("h", context());

    let levels: Vec<LogLevel> = sink.records().iter().map(|record| record.level).collect();
    assert_eq!(
        levels,
        vec![
            LogLevel::Emergency,
            LogLevel::Alert,
            LogLevel::Critical,
            LogLevel::Error,
            LogLevel::Warning,
            LogLevel::Notice,
            LogLevel::Info,
            LogLevel::Debug,
        ]
    );
    assert!(
        sink.records()
            .iter()
            .all(|record| context_value(record, "k").as_deref() == Some("v"))
    );
}

// ---- Shared context ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_context_stays_in_its_own_scope() {
    let (logger, sink) = memory_logger();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));

    let run = |tenant: &'static str| {
        let logger = logger.clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(Context::scope(ContextStore::default(), async move {
            Log::share_context(json!({"tenant": tenant}));
            // Both scopes have shared their context before either writes.
            barrier.wait().await;
            logger.info(&format!("from {tenant}"));
            barrier.wait().await;
            Log::shared_context()
        }))
    };
    let acme = run("acme");
    let globex = run("globex");
    let acme_shared = acme.await.unwrap();
    let globex_shared = globex.await.unwrap();

    assert_eq!(acme_shared.get("tenant"), Some(&json!("acme")));
    assert_eq!(globex_shared.get("tenant"), Some(&json!("globex")));
    for record in sink.records() {
        let tenant = context_value(&record, "tenant").expect("shared context on the write");
        assert_eq!(record.message, format!("from {tenant}"), "{record:?}");
    }
    assert_eq!(sink.records().len(), 2);
}

#[tokio::test]
async fn outside_a_scope_nothing_is_shared() {
    let (logger, sink) = memory_logger();

    Log::share_context(json!({"tenant": "acme"}));
    logger.info("outside");

    assert!(Log::shared_context().is_empty());
    assert_eq!(context_value(&sink.records()[0], "tenant"), None);
}

#[tokio::test]
async fn shared_context_is_merged_under_the_logger_and_the_call() {
    let (logger, sink) = memory_logger();
    Context::scope(ContextStore::default(), async {
        Log::share_context(json!({"tenant": "acme", "request": "r1", "role": "shared"}));
        Log::share_context(json!({"request": "r2"}));
        let logger = logger.with_context(json!({"role": "logger"}));
        logger.info_with("one", json!({"call": true}));

        Log::without_context(Some(&["request"]));
        logger.info("two");
        Log::flush_shared_context();
        logger.info("three");
        assert!(Log::shared_context().is_empty());
    })
    .await;

    let records = sink.records();
    assert_eq!(
        context_value(&records[0], "tenant").as_deref(),
        Some("acme")
    );
    assert_eq!(
        context_value(&records[0], "request").as_deref(),
        Some("r2"),
        "a later share replaces the key"
    );
    assert_eq!(
        context_value(&records[0], "role").as_deref(),
        Some("logger"),
        "the logger's context wins over the shared"
    );
    assert_eq!(context_value(&records[0], "call").as_deref(), Some("true"));
    assert_eq!(context_value(&records[1], "request"), None);
    assert_eq!(
        context_value(&records[1], "tenant").as_deref(),
        Some("acme")
    );
    assert_eq!(context_value(&records[2], "tenant"), None);
}

#[tokio::test]
async fn shared_context_reaches_the_default_channel() {
    let _env = crate::env_lock::lock_env_async().await;
    let _restore = EnvSnapshot::capture(VARIABLES);
    let sink = Arc::new(MemorySink::default());
    let shared = Arc::clone(&sink);
    let driver = unique("memory-default");
    Log::extend(&driver, move |_| Ok(shared.clone() as Arc<dyn LogSink>));
    let name = unique("default");
    Log::define(&name, LogChannel::driver(&driver));
    set_env("LOG_CHANNEL", Some(&name));
    suprnova::logging::check_channels().expect("LOG_CHANNEL names a channel");
    let subscriber = build_subscriber(LogConfig {
        level: "info".to_owned(),
        format: LogFormat::Pretty,
    })
    .expect("the subscriber builds");
    let _default = tracing::subscriber::set_default(subscriber);

    Context::scope(ContextStore::default(), async {
        Log::share_context(json!({"tenant": "acme"}));
        Log::info("through the facade");
        tracing::info!("through tracing");
    })
    .await;

    let records = sink.records();
    assert_eq!(records.len(), 2, "{records:?}");
    for record in &records {
        assert_eq!(
            context_value(record, "tenant").as_deref(),
            Some("acme"),
            "{record:?}"
        );
    }
    assert_eq!(records[0].level, LogLevel::Info);
    assert_eq!(records[0].message, "through the facade");
}

// ---- MessageLogged and Log::listen ---------------------------------------------

#[test]
fn log_listen_hears_a_write_to_the_null_channel() {
    let marker = unique("listened");
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&heard);
    let expected = marker.clone();
    Log::listen(move |event: &MessageLogged| {
        if event.message == expected {
            sink.lock().unwrap().push(event.clone());
        }
    });

    Log::channel("null")
        .expect("the null channel")
        .warning_with(&marker, json!({"id": 7}));

    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "one write, one call");
    assert_eq!(heard[0].level, LogLevel::Warning);
    assert_eq!(heard[0].context.get("id"), Some(&json!(7)));
}

#[test]
fn a_failing_listen_callback_does_not_fail_the_write() {
    let marker = unique("panicking");
    let expected = marker.clone();
    Log::listen(move |event: &MessageLogged| {
        if event.message == expected {
            panic!("the callback failed");
        }
    });
    let (logger, sink) = memory_logger();

    logger.error(&marker);

    assert_eq!(sink.records().len(), 1, "the write reached its channel");
}

#[test]
fn the_events_fake_records_one_message_logged_per_write() {
    let _events = EventFacade::fake();
    let marker = unique("faked");

    Log::channel("null").expect("null").info(&marker);
    Log::stack(&["null", "null"])
        .expect("a stack")
        .notice(&marker);

    let recorded = dispatched::<MessageLogged>(|event| event.message == marker);
    assert_eq!(recorded.len(), 2, "one event a write: {recorded:?}");
    assert_eq!(recorded[0].level, LogLevel::Info);
    assert_eq!(recorded[1].level, LogLevel::Notice);
}

#[test]
fn a_facade_write_records_one_message_logged_with_its_level() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("null"));
    suprnova::logging::check_channels().expect("the null channel");
    let subscriber = build_subscriber(LogConfig {
        level: "debug".to_owned(),
        format: LogFormat::Pretty,
    })
    .expect("the subscriber builds");
    let _events = EventFacade::fake();
    let marker = unique("facade");

    tracing::subscriber::with_default(subscriber, || {
        Log::critical_with(&marker, json!({"disk": "sda"}));
        tracing::warn!("{marker}");
    });

    let recorded = dispatched::<MessageLogged>(|event| event.message == marker);
    assert_eq!(recorded.len(), 2, "{recorded:?}");
    assert_eq!(recorded[0].level, LogLevel::Critical);
    assert_eq!(recorded[0].context.get("disk"), Some(&json!("sda")));
    assert_eq!(recorded[1].level, LogLevel::Warning);
}

/// Counts the `MessageLogged` events a real listener receives.
struct CountingListener {
    marker: String,
    marked: Arc<AtomicUsize>,
    dispatcher_lines: Arc<AtomicUsize>,
}

#[suprnova::async_trait]
impl Listener<MessageLogged> for CountingListener {
    async fn handle(&self, event: &MessageLogged) -> Result<(), FrameworkError> {
        if event.message == self.marker {
            self.marked.fetch_add(1, Ordering::SeqCst);
        }
        if event.message.contains("dispatching event") {
            self.dispatcher_lines.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// A listener that always fails.
struct FailingListener;

#[suprnova::async_trait]
impl Listener<MessageLogged> for FailingListener {
    async fn handle(&self, _event: &MessageLogged) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the listener failed"))
    }
}

#[test]
fn the_dispatchers_own_logging_does_not_dispatch_message_logged_again() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("null"));
    suprnova::logging::check_channels().expect("the null channel");
    let subscriber = build_subscriber(LogConfig {
        level: "debug".to_owned(),
        format: LogFormat::Pretty,
    })
    .expect("the subscriber builds");
    let marker = unique("dispatched");
    let marked = Arc::new(AtomicUsize::new(0));
    let dispatcher_lines = Arc::new(AtomicUsize::new(0));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            EventFacade::listen::<MessageLogged, _>(Arc::new(CountingListener {
                marker: marker.clone(),
                marked: Arc::clone(&marked),
                dispatcher_lines: Arc::clone(&dispatcher_lines),
            }))
            .await;
            EventFacade::listen::<MessageLogged, _>(Arc::new(FailingListener)).await;

            Log::channel("null").expect("null").info(&marker);
            Log::info(&marker);

            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while marked.load(Ordering::SeqCst) < 2 && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            // Room for any dispatch the dispatcher's own lines would start.
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
    });

    assert_eq!(
        marked.load(Ordering::SeqCst),
        2,
        "each write reaches the listener once, a failing listener beside it"
    );
    assert_eq!(
        dispatcher_lines.load(Ordering::SeqCst),
        0,
        "the dispatcher's own logging dispatched MessageLogged again"
    );
}

#[test]
fn a_write_outside_a_runtime_neither_blocks_nor_panics() {
    let marker = unique("no-runtime");
    // A real listener makes the event observed, and it fails; a write made
    // outside any runtime still returns at once.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(EventFacade::listen::<MessageLogged, _>(Arc::new(
        FailingListener,
    )));
    drop(runtime);

    Log::channel("null").expect("null").info(&marker);
}

#[test]
fn a_write_outside_a_runtime_reaches_the_dispatchers_listeners() {
    let marker = unique("outside-runtime");
    let marked = Arc::new(AtomicUsize::new(0));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(EventFacade::listen::<MessageLogged, _>(Arc::new(
        CountingListener {
            marker: marker.clone(),
            marked: Arc::clone(&marked),
            dispatcher_lines: Arc::new(AtomicUsize::new(0)),
        },
    )));
    drop(runtime);

    let written = marker.clone();
    std::thread::spawn(move || {
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "the writing thread has no runtime"
        );
        Log::channel("null")
            .expect("the null channel")
            .info(&written);
    })
    .join()
    .expect("the write returns without panicking");

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while marked.load(Ordering::SeqCst) < 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // Room for a second dispatch of the same write, which must not come.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        marked.load(Ordering::SeqCst),
        1,
        "a write made outside any runtime reaches the dispatcher's listener once"
    );
}

/// In a child: the server's subscriber at `debug`, so the dispatcher's own
/// lines on the thread that dispatches a write made outside any runtime
/// reach the layer; then a channel write and a facade write from a thread
/// with no runtime, beside a listener that fails.
#[test]
fn child_writes_outside_a_runtime_under_the_servers_subscriber() {
    if !is_child() {
        return;
    }
    as_the_server(|| {
        let marker = unique("outside-runtime-server");
        let marked = Arc::new(AtomicUsize::new(0));
        let dispatcher_lines = Arc::new(AtomicUsize::new(0));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            EventFacade::listen::<MessageLogged, _>(Arc::new(CountingListener {
                marker: marker.clone(),
                marked: Arc::clone(&marked),
                dispatcher_lines: Arc::clone(&dispatcher_lines),
            }))
            .await;
            EventFacade::listen::<MessageLogged, _>(Arc::new(FailingListener)).await;
        });
        drop(runtime);

        let written = marker.clone();
        std::thread::spawn(move || {
            Log::channel("null").expect("null").info(&written);
            Log::info(&written);
        })
        .join()
        .expect("a failing listener does not fail the write");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while marked.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // Room for any dispatch the dispatcher's own lines would start.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            marked.load(Ordering::SeqCst),
            2,
            "each write outside a runtime reaches the listener once"
        );
        assert_eq!(
            dispatcher_lines.load(Ordering::SeqCst),
            0,
            "the dispatcher's own logging dispatched MessageLogged again"
        );
    });
}

#[test]
fn a_write_outside_a_runtime_does_not_dispatch_the_dispatchers_lines() {
    run_child(
        "laravel_infra_gaps::child_writes_outside_a_runtime_under_the_servers_subscriber",
        &[("LOG_CHANNEL", "null"), ("LOG_LEVEL", "debug")],
    );
}

// ---- PAR-139: the bare level of LOG_LEVEL ---------------------------------------

#[test]
fn log_level_parse_accepts_the_psr_names_warn_and_trace() {
    for (name, level) in [
        ("emergency", LogLevel::Emergency),
        ("alert", LogLevel::Alert),
        ("critical", LogLevel::Critical),
        ("error", LogLevel::Error),
        ("warning", LogLevel::Warning),
        ("notice", LogLevel::Notice),
        ("info", LogLevel::Info),
        ("debug", LogLevel::Debug),
        ("warn", LogLevel::Warning),
        ("trace", LogLevel::Debug),
        ("WARNING", LogLevel::Warning),
    ] {
        assert_eq!(LogLevel::parse(name).expect(name), level, "{name}");
    }
    let error = LogLevel::parse("loud").expect_err("loud is no level");
    assert!(error.to_string().contains("loud"), "{error}");
}

#[test]
fn an_unknown_bare_log_level_fails_the_boot() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("null"));

    set_env("LOG_LEVEL", Some("loud"));
    let error = suprnova::logging::check_channels().expect_err("LOG_LEVEL=loud stops the boot");
    assert!(error.to_string().contains("loud"), "{error}");
    assert!(error.to_string().contains("LOG_LEVEL"), "{error}");

    set_env("LOG_LEVEL", Some("loud,sqlx=warn"));
    assert!(suprnova::logging::check_channels().is_err());

    for valid in [
        "warning",
        "critical,sqlx=warn",
        "sqlx=warn",
        "TRACE",
        "notice",
    ] {
        set_env("LOG_LEVEL", Some(valid));
        suprnova::logging::check_channels().unwrap_or_else(|error| panic!("{valid}: {error}"));
    }
}

/// In a child: write `debug` and `error` through the `errorlog` channel.
#[test]
fn child_writes_debug_and_error_to_errorlog() {
    if !is_child() {
        return;
    }
    let logger = Log::channel("errorlog").unwrap();
    logger.debug("errorlog-debug-line");
    logger.error("errorlog-error-line");
}

#[test]
fn log_level_error_keeps_debug_off_the_errorlog_channel() {
    let output = run_child(
        "laravel_infra_gaps::child_writes_debug_and_error_to_errorlog",
        &[("LOG_LEVEL", "error")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("errorlog-debug-line"), "{stderr}");
    assert!(stderr.contains("errorlog-error-line"), "{stderr}");
}

#[test]
fn an_unset_log_level_keeps_every_level_on_errorlog() {
    let output = run_child(
        "laravel_infra_gaps::child_writes_debug_and_error_to_errorlog",
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("errorlog-debug-line"), "{stderr}");
    assert!(stderr.contains("errorlog-error-line"), "{stderr}");
}

/// In a child: the server's subscriber, and `tracing` events and facade
/// writes at several levels.
#[test]
fn child_writes_through_tracing() {
    if !is_child() {
        return;
    }
    as_the_server(|| {
        tracing::error!("tracing-error-line");
        tracing::warn!("tracing-warn-line");
        tracing::info!("tracing-info-line");
        Log::critical("facade-critical-line");
        Log::error("facade-error-line");
        Log::notice("facade-notice-line");
    });
}

#[test]
fn log_level_warning_keeps_errors_on_the_default_channel() {
    let output = run_child(
        "laravel_infra_gaps::child_writes_through_tracing",
        &[("LOG_CHANNEL", "errorlog"), ("LOG_LEVEL", "warning")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("tracing-error-line"), "{stderr}");
    assert!(stderr.contains("tracing-warn-line"), "{stderr}");
    assert!(!stderr.contains("tracing-info-line"), "{stderr}");
    assert!(!stderr.contains("facade-notice-line"), "{stderr}");
}

#[test]
fn log_level_critical_keeps_critical_writes_and_drops_errors() {
    let output = run_child(
        "laravel_infra_gaps::child_writes_through_tracing",
        &[("LOG_CHANNEL", "errorlog"), ("LOG_LEVEL", "critical")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("facade-critical-line"), "{stderr}");
    assert!(!stderr.contains("facade-error-line"), "{stderr}");
    assert!(!stderr.contains("tracing-error-line"), "{stderr}");
}

#[test]
fn log_level_notice_keeps_notices_and_drops_info_in_a_file() {
    let base = tempfile::tempdir().unwrap();
    run_child(
        "laravel_infra_gaps::child_writes_through_tracing",
        &[
            ("LOG_CHANNEL", "single"),
            ("LOG_LEVEL", "notice"),
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
        ],
    );
    let text = read(&base.path().join("storage/logs/suprnova.log"));
    assert!(text.contains("NOTICE: facade-notice-line"), "{text}");
    assert!(!text.contains("tracing-info-line"), "{text}");
    assert!(text.contains("tracing-warn-line"), "{text}");
}

// ---- PAR-139: placeholders per channel -------------------------------------------

#[test]
fn a_channel_that_keeps_placeholders_writes_the_raw_message() {
    let dir = tempfile::tempdir().unwrap();
    let logger =
        Log::build(LogChannel::monthly(dir.path().join("app.log")).replace_placeholders(false))
            .expect("monthly");

    logger.info_with("user {id}", json!({"id": 7}));
    Log::flush();

    let text = read_dir(dir.path());
    assert!(text.contains("user {id}"), "{text}");
    assert!(!text.contains("user 7"), "{text}");
}

#[test]
fn a_channel_replaces_placeholders_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let logger = Log::build(LogChannel::monthly(dir.path().join("app.log"))).expect("monthly");

    logger.info_with("user {id}", json!({"id": 7}));
    Log::flush();

    assert!(read_dir(dir.path()).contains("user 7"));
}

#[test]
fn a_stack_writes_each_channels_own_form() {
    let dir = tempfile::tempdir().unwrap();
    let replacing = unique("replacing");
    let raw = unique("raw");
    Log::define(
        &replacing,
        LogChannel::single(dir.path().join("replacing.log")),
    );
    Log::define(
        &raw,
        LogChannel::single(dir.path().join("raw.log")).replace_placeholders(false),
    );

    Log::stack(&[replacing.as_str(), raw.as_str()])
        .expect("stack")
        .info_with("user {id}", json!({"id": 7}));
    Log::build(LogChannel::stack([replacing.as_str(), raw.as_str()]))
        .expect("built stack")
        .info_with("order {order}", json!({"order": 9}));
    Log::flush();

    let replaced = read(&dir.path().join("replacing.log"));
    let kept = read(&dir.path().join("raw.log"));
    assert!(
        replaced.contains("user 7") && replaced.contains("order 9"),
        "{replaced}"
    );
    assert!(
        kept.contains("user {id}") && kept.contains("order {order}"),
        "{kept}"
    );
}
