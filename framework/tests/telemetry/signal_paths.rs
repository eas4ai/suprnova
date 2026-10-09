//! Each signal is posted to its own path of the collector.
//!
//! The collector here is a listener of the test that answers every
//! request with 200 and keeps the path it was asked for. A collector
//! has the signals at `/v1/traces`, `/v1/metrics` and `/v1/logs` and
//! nothing at its root, so the path is what decides whether anything
//! arrives.

#![cfg(feature = "otel")]

use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::{LogConfig, LogFormat, Metrics, OtelConfig, init_telemetry};

/// Start the collector and return its base URL and the paths it was
/// asked for, in the order of the requests.
async fn collector() -> (String, Arc<Mutex<Vec<String>>>) {
    let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let base = format!("http://{}", listener.local_addr().expect("local_addr"));

    let recorded = asked.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let recorded = recorded.clone();
                    async move {
                        let path = request.uri().path().to_owned();
                        let _ = request.into_body().collect().await;
                        recorded
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(path);
                        Ok::<_, Infallible>(hyper::Response::new(Full::new(Bytes::new())))
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (base, asked)
}

fn config_for(base: String) -> OtelConfig {
    OtelConfig {
        endpoint: Some(base),
        disabled: false,
        ..OtelConfig::disabled()
    }
}

/// Emit one of each signal and shut the providers down. The shutdown
/// sends what the batches hold and returns when it is sent.
///
/// Returns whether the traces and the logs of this test can arrive. A
/// process has one subscriber, and the spans and the log lines go to the
/// providers of the test that installed it. Under nextest, which is how
/// the gate runs the tests, every test is a process of its own and the
/// answer is yes. In a process that another test had first, the metrics
/// are what this test can see: the meter provider is set again by every
/// `init_telemetry` that runs once the guard before it is gone.
async fn emit_and_shut_down(config: OtelConfig) -> bool {
    let has_the_subscriber = !tracing::dispatcher::has_been_set();
    // Named, and not read from the environment: with `LOG_LEVEL=warn` in
    // the shell the span and the line below would not be recorded.
    let log_config = LogConfig {
        level: "info".to_owned(),
        format: LogFormat::Pretty,
    };
    let guard = init_telemetry(log_config, config);
    {
        let span = tracing::info_span!("signal.paths.span");
        let _enter = span.enter();
        tracing::info!("a log line for the collector");
        Metrics::counter("signal.paths.counter").inc();
    }
    guard.shutdown().await;
    has_the_subscriber
}

fn paths(asked: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    asked.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn assert_posted(asked: &[String], path: &str) {
    assert!(
        asked.iter().any(|asked| asked == path),
        "nothing was posted to {path}; the collector was asked for {asked:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(telemetry_providers)]
async fn every_signal_is_posted_to_its_own_path() {
    let (base, asked) = collector().await;

    let has_the_subscriber = emit_and_shut_down(config_for(base)).await;

    let asked = paths(&asked);
    assert_posted(&asked, "/v1/metrics");
    if has_the_subscriber {
        assert_posted(&asked, "/v1/traces");
        assert_posted(&asked, "/v1/logs");
    }
    assert!(
        asked.iter().all(|path| path.starts_with("/v1/")),
        "a signal was posted outside of the paths of the signals: {asked:?}"
    );
}

/// The endpoint of one signal is a URL with a mistake in it. That signal
/// is left out, and the other two arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(telemetry_providers)]
async fn a_signal_that_cannot_be_built_does_not_take_the_others_with_it() {
    let (base, asked) = collector().await;
    let config = OtelConfig {
        traces_endpoint: Some("http://[not a url".to_owned()),
        ..config_for(base)
    };

    let has_the_subscriber = emit_and_shut_down(config).await;

    let asked = paths(&asked);
    assert!(
        !asked.iter().any(|path| path == "/v1/traces"),
        "the traces have no endpoint that can be used: {asked:?}"
    );
    assert_posted(&asked, "/v1/metrics");
    if has_the_subscriber {
        assert_posted(&asked, "/v1/logs");
    }
}

/// An endpoint that is a path alone is one the exporter would take, and
/// then send nothing to and say nothing about. It is left out as one that
/// is no URL is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(telemetry_providers)]
async fn a_signal_whose_endpoint_is_a_path_alone_is_left_out() {
    let (base, asked) = collector().await;
    let config = OtelConfig {
        logs_endpoint: Some("/v1/logs".to_owned()),
        ..config_for(base)
    };

    let has_the_subscriber = emit_and_shut_down(config).await;

    let asked = paths(&asked);
    assert!(
        !asked.iter().any(|path| path == "/v1/logs"),
        "the logs have no endpoint that can be used: {asked:?}"
    );
    assert_posted(&asked, "/v1/metrics");
    if has_the_subscriber {
        assert_posted(&asked, "/v1/traces");
    }
}

/// A signal with an endpoint of its own goes there, as it is written, and
/// the other two go to the base.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(telemetry_providers)]
async fn a_signal_with_an_endpoint_of_its_own_goes_there() {
    let (base, asked) = collector().await;
    let (other, other_asked) = collector().await;
    let config = OtelConfig {
        metrics_endpoint: Some(format!("{other}/custom/metrics")),
        ..config_for(base)
    };

    let has_the_subscriber = emit_and_shut_down(config).await;

    let asked = paths(&asked);
    assert!(
        !asked.iter().any(|path| path.contains("metrics")),
        "the metrics have an endpoint of their own: {asked:?}"
    );
    if has_the_subscriber {
        assert_posted(&asked, "/v1/traces");
        assert_posted(&asked, "/v1/logs");
    }
    let other_asked = paths(&other_asked);
    assert!(
        !other_asked.is_empty() && other_asked.iter().all(|path| path == "/custom/metrics"),
        "the endpoint of a signal is used as it is written: {other_asked:?}"
    );
}
