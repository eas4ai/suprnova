//! Log channels (PAR-026 to PAR-030).
//!
//! Most tests drive a channel directly with `Log::build`, `Log::channel` or
//! `Log::stack`, or route `tracing` events through a subscriber from
//! `suprnova::logging::build_subscriber` installed for the test's thread
//! only. The tests of the built-in channels, whose files live under the
//! storage directory, and of stdout run in a child process with its own
//! `APP_BASE_PATH`, since the paths are fixed once per process.

use serde_json::json;
use serial_test::serial;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::logging::{LogConfig, build_subscriber};
use suprnova::testing::TestClock;
use suprnova::{FrameworkError, Log, LogChannel, LogLevel, LogRecord, LogSink};

use crate::env_lock::lock_env;
use crate::env_snapshot::{EnvSnapshot, set_env};

const VARIABLES: &[&str] = &[
    "LOG_CHANNEL",
    "LOG_STACK",
    "LOG_DAILY_DAYS",
    "LOG_SYSLOG_SOCKET",
    "LOG_SYSLOG_FACILITY",
    "MAIL_LOG_CHANNEL",
    "APP_BASE_PATH",
];

/// A name no other test uses, so the process-wide channel registry never
/// mixes two tests' channels.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn at(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text).unwrap().to_utc()
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

// The child process.

const CHILD: &str = "SUPRNOVA_LOG_CHANNEL_CHILD";

fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

/// Run the test `name` of this binary alone in a child process with `env`
/// set and `LOG_CHANNEL` unset unless `env` sets it, and return its output.
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

/// In a child: install the subscriber the server installs, write one
/// event, flush, and exit.
#[test]
fn child_writes_one_event() {
    if !is_child() {
        return;
    }
    let marker = std::env::var("SUPRNOVA_LOG_MARKER").unwrap();
    let guard = suprnova::telemetry::init_telemetry(
        LogConfig::from_env(),
        suprnova::telemetry::OtelConfig::disabled(),
    );
    tracing::info!("{marker}");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // A clean shutdown flushes the file channels (PAR-029).
    runtime.block_on(guard.shutdown());
}

/// In a child: write through the built-in `daily` channel.
#[test]
fn child_writes_to_the_built_in_daily_channel() {
    if !is_child() {
        return;
    }
    let _clock = TestClock::travel_to(at("2026-09-10T12:00:00Z"));
    Log::channel("daily").unwrap().info("today");
    Log::flush();
}

// PAR-026: the default channel.

#[test]
fn with_log_channel_unset_events_go_to_stdout() {
    let marker = unique("stdout-marker");
    let output = run_child(
        "channels::child_writes_one_event",
        &[("SUPRNOVA_LOG_MARKER", &marker)],
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&marker),
        "stdout is the default channel"
    );
}

#[test]
fn log_channel_single_writes_the_storage_file_and_not_stdout() {
    let base = tempfile::tempdir().unwrap();
    let marker = unique("single-marker");
    let output = run_child(
        "channels::child_writes_one_event",
        &[
            ("SUPRNOVA_LOG_MARKER", &marker),
            ("LOG_CHANNEL", "single"),
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
        ],
    );
    let file = base.path().join("storage/logs/suprnova.log");
    assert!(
        read(&file).contains(&marker),
        "the record is in {}",
        file.display()
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(&marker),
        "and not on stdout"
    );
}

#[test]
#[serial]
fn a_defined_channel_receives_the_default_events() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let name = unique("audit");
    Log::define(&name, LogChannel::single(dir.path().join("audit.log")));
    set_env("LOG_CHANNEL", Some(&name));

    let subscriber = build_subscriber(LogConfig::from_env()).expect("a defined channel");
    tracing::subscriber::with_default(subscriber, || tracing::info!("defined-event"));
    Log::flush();
    assert!(read(&dir.path().join("audit.log")).contains("defined-event"));
}

#[test]
#[serial]
fn an_extended_driver_receives_the_records_of_a_channel_on_it() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let sink = Arc::new(MemorySink::default());
    let driver = unique("memory");
    let shared = Arc::clone(&sink);
    Log::extend(&driver, move |_channel| {
        Ok(shared.clone() as Arc<dyn LogSink>)
    });
    let name = unique("kept");
    Log::define(&name, LogChannel::driver(&driver));
    set_env("LOG_CHANNEL", Some(&name));

    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    tracing::subscriber::with_default(subscriber, || tracing::warn!(user = 7, "extended-event"));
    let records = sink.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message, "extended-event");
    assert_eq!(records[0].level, LogLevel::Warning);
    assert!(
        records[0]
            .context
            .iter()
            .any(|(key, value)| key == "user" && value == "7"),
        "{:?}",
        records[0].context
    );
}

#[test]
#[serial]
fn a_channel_that_does_not_exist_fails_boot_naming_it() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("nosuch"));
    let error = build_subscriber(LogConfig::from_env())
        .err()
        .expect("an unknown LOG_CHANNEL fails boot");
    assert!(error.to_string().contains("nosuch"), "{error}");

    set_env("LOG_CHANNEL", Some("stack"));
    set_env("LOG_STACK", Some("stderr,nope"));
    let error = build_subscriber(LogConfig::from_env())
        .err()
        .expect("an unknown LOG_STACK entry fails boot");
    assert!(error.to_string().contains("nope"), "{error}");
}

// PAR-027: files, rotation and retention.

#[test]
fn single_appends_each_record_and_creates_the_directories() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("deep/er/app.log");
    let logger = Log::build(LogChannel::single(&file)).unwrap();
    logger.info("one");
    logger.info("two");
    Log::flush();

    let text = read(&file);
    let one = text.find("one").expect("first record");
    let two = text.find("two").expect("second record");
    assert!(one < two, "appended in order: {text}");
}

#[test]
fn daily_writes_one_file_a_day_by_the_framework_clock() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::travel_to(at("2026-10-02T23:59:59Z"));
    let logger = Log::build(LogChannel::daily(dir.path().join("app.log"))).unwrap();
    logger.info("before-midnight");
    clock.advance(chrono::Duration::seconds(2));
    logger.info("after-midnight");
    Log::flush();

    assert!(read(&dir.path().join("app-2026-10-02.log")).contains("before-midnight"));
    let next = read(&dir.path().join("app-2026-10-03.log"));
    assert!(next.contains("after-midnight"));
    assert!(!next.contains("before-midnight"));
}

#[test]
fn daily_keeps_the_newest_days() {
    let dir = tempfile::tempdir().unwrap();
    for day in 1..=20 {
        std::fs::write(
            dir.path().join(format!("app-2026-09-{day:02}.log")),
            "old\n",
        )
        .unwrap();
    }
    std::fs::write(dir.path().join("unrelated.txt"), "keep me").unwrap();
    let _clock = TestClock::travel_to(at("2026-09-21T08:00:00Z"));
    let logger = Log::build(LogChannel::daily(dir.path().join("app.log")).days(14)).unwrap();
    logger.info("today");
    Log::flush();

    let logs: Vec<String> = files_in(dir.path())
        .into_iter()
        .filter(|name| name.starts_with("app-"))
        .collect();
    assert_eq!(logs.len(), 14, "{logs:?}");
    assert_eq!(logs.first().unwrap(), "app-2026-09-08.log");
    assert_eq!(logs.last().unwrap(), "app-2026-09-21.log");
    assert!(
        dir.path().join("unrelated.txt").exists(),
        "other files are left alone"
    );
}

#[test]
fn the_built_in_daily_channel_reads_log_daily_days() {
    let base = tempfile::tempdir().unwrap();
    let logs = base.path().join("storage/logs");
    std::fs::create_dir_all(&logs).unwrap();
    for day in 1..=5 {
        std::fs::write(logs.join(format!("suprnova-2026-09-{day:02}.log")), "old\n").unwrap();
    }
    run_child(
        "channels::child_writes_to_the_built_in_daily_channel",
        &[
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
            ("LOG_DAILY_DAYS", "3"),
        ],
    );
    assert_eq!(
        files_in(&logs),
        [
            "suprnova-2026-09-04.log",
            "suprnova-2026-09-05.log",
            "suprnova-2026-09-10.log"
        ]
    );
}

#[test]
fn monthly_writes_one_file_a_month_and_keeps_three() {
    let dir = tempfile::tempdir().unwrap();
    for month in 5..=9 {
        std::fs::write(dir.path().join(format!("app-2026-{month:02}.log")), "old\n").unwrap();
    }
    let _clock = TestClock::travel_to(at("2026-10-02T08:00:00Z"));
    let logger = Log::build(LogChannel::monthly(dir.path().join("app.log"))).unwrap();
    logger.info("this-month");
    Log::flush();

    assert_eq!(
        files_in(dir.path()),
        ["app-2026-08.log", "app-2026-09.log", "app-2026-10.log"]
    );
    assert!(read(&dir.path().join("app-2026-10.log")).contains("this-month"));
}

// PAR-028: stacks, the facade, placeholders and the mail channel.

#[test]
fn a_stack_writes_to_every_channel_and_a_broken_one_stops_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a-file"), "").unwrap();
    let (first, broken, last) = (unique("first"), unique("broken"), unique("last"));
    Log::define(&first, LogChannel::single(dir.path().join("first.log")));
    // A directory cannot be made under a file, so this channel cannot open.
    Log::define(
        &broken,
        LogChannel::single(dir.path().join("a-file/x/broken.log")),
    );
    Log::define(&last, LogChannel::single(dir.path().join("last.log")));

    let logger = Log::stack(&[first.as_str(), broken.as_str(), last.as_str()]).unwrap();
    logger.info("stacked");
    Log::flush();

    assert!(read(&dir.path().join("first.log")).contains("stacked"));
    assert!(read(&dir.path().join("last.log")).contains("stacked"));
}

#[test]
#[serial]
fn a_stack_default_channel_writes_each_event_to_every_channel() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (unique("one"), unique("two"));
    Log::define(&one, LogChannel::single(dir.path().join("one.log")));
    Log::define(&two, LogChannel::single(dir.path().join("two.log")));
    set_env("LOG_CHANNEL", Some("stack"));
    set_env("LOG_STACK", Some(&format!("{one},{two}")));

    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    tracing::subscriber::with_default(subscriber, || tracing::error!("both-files"));
    assert!(read(&dir.path().join("one.log")).contains("both-files"));
    assert!(read(&dir.path().join("two.log")).contains("both-files"));
}

#[test]
#[serial]
fn log_channel_writes_to_that_channel_only() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let (default, other) = (unique("default"), unique("other"));
    Log::define(&default, LogChannel::single(dir.path().join("default.log")));
    Log::define(&other, LogChannel::single(dir.path().join("other.log")));
    set_env("LOG_CHANNEL", Some(&default));

    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    tracing::subscriber::with_default(subscriber, || {
        Log::channel(&other).unwrap().info("only-other");
    });
    Log::flush();
    assert!(read(&dir.path().join("other.log")).contains("only-other"));
    assert!(!read(&dir.path().join("default.log")).contains("only-other"));
}

#[test]
fn log_build_writes_to_a_channel_with_no_name() {
    let dir = tempfile::tempdir().unwrap();
    let logger = Log::build(LogChannel::single(dir.path().join("built.log"))).unwrap();
    logger.warning("built-record");
    Log::flush();
    assert!(read(&dir.path().join("built.log")).contains("built-record"));
}

#[test]
fn placeholders_are_replaced_from_the_context() {
    let dir = tempfile::tempdir().unwrap();
    let logger = Log::build(LogChannel::single(dir.path().join("p.log"))).unwrap();
    logger.info_with(
        "user {id} did {what} at {missing}",
        json!({"id": 7, "what": "login"}),
    );
    Log::flush();
    let text = read(&dir.path().join("p.log"));
    assert!(text.contains("user 7 did login at {missing}"), "{text}");
}

#[test]
fn the_eight_levels_are_written_with_their_names() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("levels.log");
    let logger = Log::build(LogChannel::single(&file).level(LogLevel::Debug)).unwrap();
    logger.emergency("m");
    logger.alert("m");
    logger.critical("m");
    logger.error("m");
    logger.warning("m");
    logger.notice("m");
    logger.info("m");
    logger.debug("m");
    Log::flush();
    let text = read(&file);
    for name in [
        "EMERGENCY",
        "ALERT",
        "CRITICAL",
        "ERROR",
        "WARNING",
        "NOTICE",
        "INFO",
        "DEBUG",
    ] {
        assert!(text.contains(name), "{name} in {text}");
    }
}

#[test]
fn a_channel_level_drops_the_records_below_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("level.log");
    let logger = Log::build(LogChannel::single(&file).level(LogLevel::Warning)).unwrap();
    logger.info("too-low");
    logger.error("high-enough");
    Log::flush();
    let text = read(&file);
    assert!(
        !text.contains("too-low") && text.contains("high-enough"),
        "{text}"
    );
}

#[test]
#[serial]
fn the_channels_in_use_and_the_default_are_reported_and_changed() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (unique("first"), unique("second"));
    Log::define(&first, LogChannel::single(dir.path().join("first.log")));
    Log::define(&second, LogChannel::single(dir.path().join("second.log")));
    set_env("LOG_CHANNEL", Some(&first));

    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    assert_eq!(Log::default_channel(), first);
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!("before-switch");
        Log::set_default_channel(&second).unwrap();
        tracing::error!("after-switch");
    });
    assert_eq!(Log::default_channel(), second);
    assert!(Log::set_default_channel("no-such-channel").is_err());

    let first_text = read(&dir.path().join("first.log"));
    let second_text = read(&dir.path().join("second.log"));
    assert!(first_text.contains("before-switch") && !first_text.contains("after-switch"));
    assert!(second_text.contains("after-switch") && !second_text.contains("before-switch"));

    Log::channel(&second).unwrap();
    assert!(Log::channels().contains(&second));
    Log::forget_channel(&second);
    assert!(!Log::channels().contains(&second));
}

#[test]
#[serial]
fn mail_log_channel_names_the_channel_the_log_mail_transport_writes_to() {
    use suprnova::mail::log::LogMailTransport;
    use suprnova::mail::transport::{MailTransport, OutgoingMessage};

    let dir = tempfile::tempdir().unwrap();
    let name = unique("mail");
    Log::define(&name, LogChannel::single(dir.path().join("mail.log")));
    {
        let _env = lock_env();
        let _restore = EnvSnapshot::capture(VARIABLES);
        set_env("MAIL_LOG_CHANNEL", Some(&name));
        let mut message = OutgoingMessage::new("from@example.com".into());
        message.to.push("to@example.com".into());
        message.subject = "Your receipt".into();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(LogMailTransport.send(&message))
            .unwrap();
    }
    Log::flush();
    assert!(read(&dir.path().join("mail.log")).contains("Your receipt"));
}

// PAR-029: flushing.

#[test]
fn log_flush_writes_a_buffered_record() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("flush.log");
    let logger = Log::build(LogChannel::single(&file)).unwrap();
    logger.info("buffered");
    Log::flush();
    assert!(read(&file).contains("buffered"));
}

#[test]
fn an_error_record_is_in_the_file_when_the_call_returns() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("error.log");
    Log::build(LogChannel::single(&file))
        .unwrap()
        .error("at-once");
    assert!(read(&file).contains("at-once"));
}

#[test]
fn an_info_record_reaches_the_file_within_a_second() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("soon.log");
    Log::build(LogChannel::single(&file)).unwrap().info("soon");
    std::thread::sleep(Duration::from_millis(2000));
    assert!(read(&file).contains("soon"));
}

#[test]
fn a_record_written_before_a_clean_shutdown_is_in_the_file() {
    // The child writes one info event and shuts the telemetry guard down
    // at once, before the one-second flush could run.
    let base = tempfile::tempdir().unwrap();
    let marker = unique("shutdown-marker");
    run_child(
        "channels::child_writes_one_event",
        &[
            ("SUPRNOVA_LOG_MARKER", &marker),
            ("LOG_CHANNEL", "single"),
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
        ],
    );
    assert!(read(&base.path().join("storage/logs/suprnova.log")).contains(&marker));
}

// PAR-030: syslog.

#[cfg(unix)]
#[test]
fn syslog_sends_rfc3164_datagrams_with_the_priority() {
    use std::os::unix::net::UnixDatagram;

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("log.sock");
    let socket = UnixDatagram::bind(&socket_path).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let logger = Log::build(
        LogChannel::syslog()
            .socket(&socket_path)
            .facility("local3")
            .level(LogLevel::Debug),
    )
    .unwrap();
    logger.warning("syslog-warning");
    let mut buffer = [0u8; 2048];
    let read = socket.recv(&mut buffer).unwrap();
    let datagram = String::from_utf8_lossy(&buffer[..read]).into_owned();
    // local3 is facility 19, warning severity 4: 19 * 8 + 4.
    assert!(datagram.starts_with("<156>"), "{datagram}");
    assert!(datagram.contains("syslog-warning"), "{datagram}");

    let default = Log::build(LogChannel::syslog().socket(&socket_path)).unwrap();
    default.error("syslog-error");
    let read = socket.recv(&mut buffer).unwrap();
    // user is facility 1, error severity 3.
    assert!(String::from_utf8_lossy(&buffer[..read]).starts_with("<11>"));
}

#[test]
#[serial]
fn an_unknown_syslog_facility_fails() {
    let error = Log::build(LogChannel::syslog().facility("nonsense"))
        .err()
        .expect("an unknown facility");
    assert!(error.to_string().contains("nonsense"), "{error}");

    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("syslog"));
    set_env("LOG_SYSLOG_FACILITY", Some("bogus"));
    let error: FrameworkError = build_subscriber(LogConfig::from_env())
        .err()
        .expect("an unknown LOG_SYSLOG_FACILITY fails boot");
    assert!(error.to_string().contains("bogus"), "{error}");
}

// Review fixes.

/// In a child: the default channel is a stack of both standard streams;
/// one `error!` callsite writes before and after the default moves to a
/// file, so the callsite's cached interest is what is tested.
#[test]
fn child_moves_the_default_off_the_standard_streams() {
    if !is_child() {
        return;
    }
    let file = std::env::var("SUPRNOVA_LOG_FILE").unwrap();
    let guard = suprnova::telemetry::init_telemetry(
        LogConfig::from_env(),
        suprnova::telemetry::OtelConfig::disabled(),
    );
    for phase in ["before", "after"] {
        tracing::error!("{phase}-switch-marker");
        if phase == "before" {
            Log::define("switched", LogChannel::single(&file));
            Log::set_default_channel("switched").unwrap();
        }
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(guard.shutdown());
}

#[test]
fn set_default_channel_moves_events_off_stdout_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("switched.log");
    let output = run_child(
        "channels::child_moves_the_default_off_the_standard_streams",
        &[
            ("LOG_CHANNEL", "stack"),
            ("LOG_STACK", "stdout,stderr"),
            ("SUPRNOVA_LOG_FILE", file.to_str().unwrap()),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("before-switch-marker") && stderr.contains("before-switch-marker"));
    assert!(
        !stdout.contains("after-switch-marker"),
        "stdout after the switch:\n{stdout}"
    );
    assert!(
        !stderr.contains("after-switch-marker"),
        "stderr after the switch:\n{stderr}"
    );
    let text = read(&file);
    assert!(text.contains("after-switch-marker") && !text.contains("before-switch-marker"));
}

#[test]
#[serial]
fn the_log_variables_are_checked_whether_or_not_the_default_uses_them() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    set_env("LOG_CHANNEL", Some("stdout"));

    set_env("LOG_STACK", Some("stdout,nope"));
    let error = build_subscriber(LogConfig::from_env())
        .err()
        .expect("LOG_STACK");
    assert!(error.to_string().contains("nope"), "{error}");
    set_env("LOG_STACK", None);

    set_env("LOG_SYSLOG_FACILITY", Some("bogus"));
    let error = build_subscriber(LogConfig::from_env())
        .err()
        .expect("LOG_SYSLOG_FACILITY");
    assert!(error.to_string().contains("bogus"), "{error}");
    set_env("LOG_SYSLOG_FACILITY", None);

    set_env("LOG_DAILY_DAYS", Some("abc"));
    let error = build_subscriber(LogConfig::from_env())
        .err()
        .expect("LOG_DAILY_DAYS");
    assert!(error.to_string().contains("abc"), "{error}");
}

#[test]
fn zero_days_keeps_every_daily_file() {
    let dir = tempfile::tempdir().unwrap();
    for day in 1..=20 {
        std::fs::write(
            dir.path().join(format!("app-2026-09-{day:02}.log")),
            "old\n",
        )
        .unwrap();
    }
    let _clock = TestClock::travel_to(at("2026-09-21T08:00:00Z"));
    Log::build(LogChannel::daily(dir.path().join("app.log")).days(0))
        .unwrap()
        .info("today");
    Log::flush();
    assert_eq!(
        files_in(dir.path()).len(),
        21,
        "0 keeps every file, as in Laravel"
    );
}

#[test]
fn log_daily_days_zero_keeps_every_file_of_the_built_in_channel() {
    let base = tempfile::tempdir().unwrap();
    let logs = base.path().join("storage/logs");
    std::fs::create_dir_all(&logs).unwrap();
    for day in 1..=5 {
        std::fs::write(logs.join(format!("suprnova-2026-09-{day:02}.log")), "old\n").unwrap();
    }
    run_child(
        "channels::child_writes_to_the_built_in_daily_channel",
        &[
            ("APP_BASE_PATH", base.path().to_str().unwrap()),
            ("LOG_DAILY_DAYS", "0"),
        ],
    );
    assert_eq!(files_in(&logs).len(), 6);
}

#[test]
fn a_placeholder_key_can_hold_any_character() {
    let dir = tempfile::tempdir().unwrap();
    let logger = Log::build(LogChannel::single(dir.path().join("keys.log"))).unwrap();
    logger.info_with(
        "user {user-id} in {the team}",
        json!({"user-id": 7, "the team": "ops"}),
    );
    Log::flush();
    assert!(read(&dir.path().join("keys.log")).contains("user 7 in ops"));
}

#[test]
#[serial]
fn a_file_line_carries_the_fields_of_its_spans() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let name = unique("spans");
    Log::define(&name, LogChannel::single(dir.path().join("spans.log")));
    set_env("LOG_CHANNEL", Some(&name));

    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    tracing::subscriber::with_default(subscriber, || {
        let request = tracing::info_span!(
            "request",
            request_id = "req-123",
            user = tracing::field::Empty
        );
        let _entered = request.enter();
        request.record("user", 7);
        tracing::info!(step = "charge", "in-span");
    });
    Log::flush();
    let text = read(&dir.path().join("spans.log"));
    assert!(text.contains("in-span"), "{text}");
    assert!(text.contains("req-123"), "the request span's id: {text}");
    assert!(
        text.contains("\"user\":\"7\""),
        "a recorded span field: {text}"
    );
    assert!(text.contains("charge"), "the event's own field: {text}");
}

#[test]
#[serial]
fn redefining_or_forgetting_the_default_channel_moves_its_events() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let name = unique("moving");
    Log::define(&name, LogChannel::single(dir.path().join("first.log")));
    set_env("LOG_CHANNEL", Some(&name));
    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        tracing::error!("to-first");
        Log::define(&name, LogChannel::single(dir.path().join("second.log")));
        tracing::error!("to-second");
        // A file rotated away is reopened once the channel is forgotten.
        std::fs::rename(
            dir.path().join("second.log"),
            dir.path().join("rotated.log"),
        )
        .unwrap();
        Log::forget_channel(&name);
        tracing::error!("reopened");
    });
    assert!(read(&dir.path().join("first.log")).contains("to-first"));
    assert!(read(&dir.path().join("rotated.log")).contains("to-second"));
    assert!(read(&dir.path().join("second.log")).contains("reopened"));
}

/// A driver sink that counts its flushes.
#[derive(Default)]
struct CountingSink {
    flushes: std::sync::atomic::AtomicUsize,
}

impl LogSink for CountingSink {
    fn write(&self, _record: &LogRecord) -> std::io::Result<()> {
        Ok(())
    }

    fn flush(&self) -> std::io::Result<()> {
        self.flushes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn log_flush_reaches_a_driver_sink() {
    let sink = Arc::new(CountingSink::default());
    let driver = unique("counting");
    let shared = Arc::clone(&sink);
    Log::extend(&driver, move |_| Ok(shared.clone() as Arc<dyn LogSink>));
    let logger = Log::build(LogChannel::driver(&driver)).unwrap();
    logger.info("buffered-by-a-driver");
    Log::flush();
    assert!(sink.flushes.load(std::sync::atomic::Ordering::SeqCst) >= 1);
}

#[suprnova::command(
    name = "log:write-and-exit",
    description = "Writes one record to a file channel"
)]
async fn write_and_exit(_args: Vec<String>) -> Result<(), FrameworkError> {
    let file = std::env::var("SUPRNOVA_LOG_FILE").unwrap();
    Log::build(LogChannel::single(file))?.info("console-marker");
    Ok(())
}

/// In a child: run a console command, then exit at once, with no
/// destructor and no time for the flusher.
#[test]
fn child_runs_a_console_command_and_exits() {
    if !is_child() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(suprnova::console::dispatch_argv(vec![
            "console".into(),
            "log:write-and-exit".into(),
        ]))
        .unwrap();
    std::process::exit(0);
}

#[test]
fn a_console_command_flushes_its_records_before_the_process_exits() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("console.log");
    run_child(
        "channels::child_runs_a_console_command_and_exits",
        &[("SUPRNOVA_LOG_FILE", file.to_str().unwrap())],
    );
    assert!(read(&file).contains("console-marker"));
}

// Audit fixes (DRIVERS-028 to DRIVERS-033).

/// A driver sink whose writes fail with `marker`.
struct FailingWrites(&'static str);

impl LogSink for FailingWrites {
    fn write(&self, _record: &LogRecord) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0))
    }
}

/// A driver sink that takes every record and then fails to flush them.
struct FailingFlushes(&'static str);

impl LogSink for FailingFlushes {
    fn write(&self, _record: &LogRecord) -> std::io::Result<()> {
        Ok(())
    }

    fn flush(&self) -> std::io::Result<()> {
        Err(std::io::Error::other(self.0))
    }
}

/// In a child: write through a driver whose writes fail, a driver whose
/// flushes fail, and (on Linux) a file on a full device, twice each.
#[test]
fn child_writes_through_sinks_that_fail() {
    if !is_child() {
        return;
    }
    Log::extend("failing-writes", |_| {
        Ok(Arc::new(FailingWrites("write-refused-marker")) as Arc<dyn LogSink>)
    });
    Log::extend("failing-flushes", |_| {
        Ok(Arc::new(FailingFlushes("flush-refused-marker")) as Arc<dyn LogSink>)
    });
    let writes = Log::build(LogChannel::driver("failing-writes")).unwrap();
    let flushes = Log::build(LogChannel::driver("failing-flushes")).unwrap();
    #[cfg(target_os = "linux")]
    let full = Log::build(LogChannel::single("/dev/full")).unwrap();
    for _ in 0..2 {
        writes.info("lost");
        flushes.info("buffered");
        #[cfg(target_os = "linux")]
        full.info("buffered");
        Log::flush();
    }
}

/// DRIVERS-028: the `LogSink` contract says the caller reports a write that
/// fails, once, on stderr. A driver's failed write, a driver's failed flush
/// and a buffered file's failed flush were all dropped without a word.
#[test]
fn a_sink_that_cannot_write_or_flush_is_reported_once_on_stderr() {
    let output = run_child("channels::child_writes_through_sinks_that_fail", &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("write-refused-marker").count(),
        1,
        "a driver's failed write, once: {stderr}"
    );
    assert_eq!(
        stderr.matches("flush-refused-marker").count(),
        1,
        "a driver's failed flush, once: {stderr}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        stderr.matches("/dev/full").count(),
        1,
        "a file whose buffered records cannot be flushed, once: {stderr}"
    );
}

/// DRIVERS-029: a stack's own `.level(...)` applies to every channel it
/// lists, on top of each channel's own level.
#[test]
#[serial]
fn a_stack_level_drops_the_records_below_it_in_every_channel_it_lists() {
    let _env = lock_env();
    let _restore = EnvSnapshot::capture(VARIABLES);
    let dir = tempfile::tempdir().unwrap();
    let (open, strict, stacked) = (unique("open"), unique("strict"), unique("stacked"));
    Log::define(&open, LogChannel::single(dir.path().join("open.log")));
    Log::define(
        &strict,
        LogChannel::single(dir.path().join("strict.log")).level(LogLevel::Error),
    );
    let stack = || LogChannel::stack([open.as_str(), strict.as_str()]).level(LogLevel::Warning);

    let logger = Log::build(stack()).unwrap();
    logger.info("built-info");
    logger.warning("built-warning");
    logger.error("built-error");

    Log::define(&stacked, stack());
    set_env("LOG_CHANNEL", Some(&stacked));
    let subscriber = build_subscriber(LogConfig::from_env()).unwrap();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("default-info");
        tracing::warn!("default-warning");
    });
    Log::flush();

    let open_text = read(&dir.path().join("open.log"));
    let strict_text = read(&dir.path().join("strict.log"));
    assert!(
        !open_text.contains("built-info") && !open_text.contains("default-info"),
        "the stack's Warning drops info in a channel that keeps every level: {open_text}"
    );
    assert!(open_text.contains("built-warning") && open_text.contains("default-warning"));
    assert!(
        !strict_text.contains("built-warning") && strict_text.contains("built-error"),
        "a channel's own stricter level still applies: {strict_text}"
    );
}
