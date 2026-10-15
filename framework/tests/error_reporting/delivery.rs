use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use suprnova::{FrameworkError, ReportingConfig, ReportingEvent};

#[inline(never)]
fn reporting_latency_original_site(error: &FrameworkError) -> Duration {
    let started = Instant::now();
    suprnova::Reporting::report(error).expect("report through facade");
    started.elapsed()
}

#[test]
#[serial_test::serial]
fn report_005_cold_sentry_capture_defers_symbols_and_keeps_the_reporting_site() {
    const CHILD: &str = "SUPRNOVA_REPORTING_COLD_STACK_TEST";
    if std::env::var_os(CHILD).is_none() {
        let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "delivery::report_005_cold_sentry_capture_defers_symbols_and_keeps_the_reporting_site", "--nocapture"])
            .env(CHILD, "1")
            .env_remove("RUST_BACKTRACE")
            .output().expect("cold reporting process");
        assert!(
            child.status.success(),
            "cold capture failed: {}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    }
    let (dsn, received) = receiver();
    suprnova::Reporting::configure(ReportingConfig::default().dsn(dsn)).expect("configure");
    let elapsed = reporting_latency_original_site(&FrameworkError::internal("cold symbol capture"));
    assert!(
        elapsed < Duration::from_millis(150),
        "report resolved stack symbols before queue admission: {elapsed:?}"
    );
    assert!(suprnova::Reporting::flush(Duration::from_secs(5)).expect("flush"));
    let body = received
        .recv_timeout(Duration::from_secs(1))
        .expect("event");
    let payload: serde_json::Value = serde_json::from_slice(
        body.split(|byte| *byte == b'\n')
            .nth(2)
            .expect("event payload"),
    )
    .expect("event JSON");
    let frames = payload["exception"]["values"][0]["stacktrace"]["frames"]
        .as_array()
        .expect("frames");
    assert!(
        frames.iter().any(|frame| frame["function"]
            .as_str()
            .is_some_and(|name| name.contains("reporting_latency_original_site"))),
        "captured stack lost its reporting function: {frames:?}"
    );
    assert_eq!(
        suprnova::Reporting::metrics().expect("metrics").delivered,
        1
    );
}

#[derive(Default)]
struct CompleteEvents(std::sync::Mutex<Vec<ReportingEvent>>);

impl suprnova::Reporter for CompleteEvents {
    fn report(&self, event: ReportingEvent) -> Result<(), FrameworkError> {
        self.0.lock().expect("captured events").push(event);
        Ok(())
    }

    fn flush(&self, _: Duration) -> Result<bool, FrameworkError> {
        Ok(true)
    }
}

#[test]
#[serial_test::serial]
fn report_003_custom_reporter_receives_resolved_frames_through_the_facade() {
    let capture = std::sync::Arc::new(CompleteEvents::default());
    suprnova::App::bind::<dyn suprnova::Reporter>(capture.clone());
    reporting_latency_original_site(&FrameworkError::internal("complete custom event"));
    let events = capture.0.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .payload
            .to_string()
            .contains("reporting_latency_original_site")
    );
}

#[inline(never)]
fn reporting_hook_original_site() {
    panic!("cold hook capture");
}

#[test]
#[serial_test::serial]
fn report_005_cold_panic_hook_defers_symbols_and_keeps_the_panic_site() {
    const CHILD: &str = "SUPRNOVA_REPORTING_COLD_HOOK_TEST";
    if std::env::var_os(CHILD).is_none() {
        let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "delivery::report_005_cold_panic_hook_defers_symbols_and_keeps_the_panic_site",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("RUST_BACKTRACE")
            .output()
            .expect("cold panic process");
        assert!(
            child.status.success(),
            "cold hook failed: {}{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    }
    suprnova::Exceptions::reset();
    suprnova::Reporting::configure(
        ReportingConfig::default().driver(suprnova::ReportingDriver::Null),
    )
    .expect("configure");
    let capture = std::sync::Arc::new(CompleteEvents::default());
    suprnova::App::bind::<dyn suprnova::Reporter>(capture.clone());
    suprnova::Reporting::bootstrap().expect("bind hook");
    let started = Instant::now();
    assert!(
        std::thread::spawn(reporting_hook_original_site)
            .join()
            .is_err()
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(150),
        "panic hook resolved stack symbols before queue admission: {elapsed:?}"
    );
    assert!(suprnova::Reporting::flush(Duration::from_secs(5)).expect("flush"));
    let events = capture.0.lock().expect("events");
    assert_eq!(events.len(), 1);
    let payload = events[0].payload.to_string();
    assert!(payload.contains("reporting_hook_original_site"));
    let frames = events[0].payload["exception"]["values"][0]["stacktrace"]["frames"]
        .as_array()
        .expect("panic frames");
    let site = frames.last().expect("panic site");
    assert!(
        site["filename"]
            .as_str()
            .is_some_and(|file| file.ends_with("delivery.rs"))
    );
    assert!(site["lineno"].as_u64().is_some_and(|line| line > 0));
}

pub(crate) fn receiver() -> (String, Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test receiver");
    let address = listener.local_addr().expect("receiver address");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read deadline");
            let mut input = BufReader::new(&mut stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                if input.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().expect("content length");
                }
            }
            let mut body = vec![0; length];
            input.read_exact(&mut body).expect("envelope body");
            drop(input);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .expect("response");
            if sender.send(body).is_err() {
                return;
            }
        }
    });
    (format!("http://public@{address}/1"), receiver)
}

#[test]
#[serial_test::serial]
fn report_001_sentry_driver_sends_an_official_envelope_to_the_dsn() {
    let (dsn, received) = receiver();
    let reporter = ReportingConfig::default()
        .dsn(dsn)
        .build()
        .expect("reporter");
    reporter
        .report(ReportingEvent::from_error(&FrameworkError::internal(
            "envelope check",
        )))
        .expect("report");
    assert!(reporter.flush(Duration::from_secs(3)).expect("flush"));
    let body = String::from_utf8(
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("event received"),
    )
    .expect("utf8 envelope");
    let lines: Vec<_> = body.lines().collect();
    let header: serde_json::Value = serde_json::from_str(lines[1]).expect("item header");
    assert_eq!(header["type"], "event");
    assert!(body.contains("envelope check"));
}

#[test]
#[serial_test::serial]
fn report_005_duplicate_burst_sends_once_and_counts_suppression() {
    let (dsn, received) = receiver();
    let reporter = ReportingConfig::default()
        .dsn(dsn)
        .build()
        .expect("reporter");
    for _ in 0..3 {
        reporter
            .report_error(&FrameworkError::internal("same incident"))
            .expect("report");
    }
    assert!(reporter.flush(Duration::from_secs(3)).expect("flush"));
    assert!(received.recv_timeout(Duration::from_secs(1)).is_ok());
    assert!(received.try_recv().is_err());
    assert_eq!(reporter.metrics().duplicates, 2);
    assert_eq!(reporter.metrics().delivered, 1);
}

#[test]
#[serial_test::serial]
fn report_005_unreachable_tracker_is_counted_and_push_does_not_wait_for_network() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("unused address");
    let address = listener.local_addr().expect("address");
    drop(listener);
    let reporter = ReportingConfig::default()
        .dsn(format!("http://public@{address}/1"))
        .timeout(Duration::from_millis(100))
        .build()
        .expect("reporter");
    let error = FrameworkError::internal("offline");
    let start = Instant::now();
    reporter.report_error(&error).expect("report");
    assert!(
        start.elapsed() < Duration::from_millis(200),
        "queue push waited on the tracker"
    );
    assert!(reporter.flush(Duration::from_secs(2)).expect("flush"));
    assert_eq!(reporter.metrics().dropped, 1);
}

#[test]
#[serial_test::serial]
fn report_005_full_queue_and_flush_have_a_fixed_bound() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("stalled receiver");
    let address = listener.local_addr().expect("address");
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        accepted_tx.send(()).expect("accepted");
        release_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("release response");
        let _ = stream.write_all(
            b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    });
    let reporter = ReportingConfig::default()
        .dsn(format!("http://public@{address}/1"))
        .queue_capacity(1)
        .timeout(Duration::from_secs(1))
        .build()
        .expect("reporter");
    reporter
        .report_error(&FrameworkError::internal("first"))
        .expect("push");
    accepted_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("sender started");
    reporter
        .report_error(&FrameworkError::internal("second"))
        .expect("push");
    reporter
        .report_error(&FrameworkError::internal("third"))
        .expect("push");
    assert_eq!(reporter.metrics().dropped, 1);
    let start = Instant::now();
    assert!(
        !reporter
            .flush(Duration::from_millis(20))
            .expect("bounded flush")
    );
    assert!(start.elapsed() < Duration::from_millis(200));
    release_tx.send(()).expect("release sender");
    assert!(reporter.flush(Duration::from_secs(3)).expect("drain"));
}
