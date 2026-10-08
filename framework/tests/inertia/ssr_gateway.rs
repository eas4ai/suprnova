//! PAR-057 to PAR-060: the SSR gateway - SSR on by default behind bundle
//! detection, hot mode through the Vite dev server, the worker's error
//! answer and the `SsrRenderFailed` event, `https` workers, and the
//! gateway as a binding the `Inertia` facade acts on.
//!
//! Laravel's references are `Inertia\Ssr\HttpGateway`, `BundleDetector`,
//! `SsrRenderFailed`, `SsrErrorType` and `SsrException` in
//! inertia-laravel 3.5.1. Every worker here is a fake: a tokio listener
//! the test drives, answering `/render`, `/__inertia_ssr` and `/health`
//! with the status and body the test names, or closing without an answer.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use suprnova::{FrameworkError, InertiaConfig, InertiaResponse, SsrConfig};

use crate::protocol_harness::MockReq;

/// The body a worker that renders answers with.
const RENDERED: &str = "<div data-server-rendered=\"true\" id=\"app\">rendered</div>";

/// One request a fake worker received.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    body: Vec<u8>,
}

/// A fake SSR worker: it answers every request with one status and body
/// and records what it was sent.
struct Worker {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Worker {
    /// A worker that renders: `200` with a head and a body.
    async fn rendering() -> Self {
        let answer = json!({
            "head": ["<title>From the worker</title>"],
            "body": RENDERED,
        });
        Self::answering(200, answer.to_string()).await
    }

    /// A worker that answers every request with `status` and `body`.
    async fn answering(status: u16, body: impl Into<Bytes>) -> Self {
        let body: Bytes = body.into();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let recorded = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                        let body = body.clone();
                        let recorded = recorded.clone();
                        async move {
                            let (parts, incoming) = req.into_parts();
                            let sent = incoming
                                .collect()
                                .await
                                .map(|collected| collected.to_bytes().to_vec())
                                .unwrap_or_default();
                            recorded.lock().unwrap().push(Seen {
                                method: parts.method.to_string(),
                                path: parts.uri.path().to_string(),
                                body: sent,
                            });
                            Ok::<_, Infallible>(
                                hyper::Response::builder()
                                    .status(status)
                                    .header("content-type", "application/json")
                                    .body(Full::new(body))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), svc)
                        .await;
                });
            }
        });
        Self { addr, seen }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// An address nothing listens on: a port the system handed out and that
/// was closed again.
async fn closed_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// The messages the `on_error` hook received, and a config carrying it.
fn with_error_hook(config: InertiaConfig) -> (InertiaConfig, Arc<Mutex<Vec<String>>>) {
    let messages: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = messages.clone();
    let config = config.on_ssr_error(move |message| sink.lock().unwrap().push(message.to_string()));
    (config, messages)
}

/// A first visit to `/dashboard` rendering `Dashboard` with `config`: the
/// document, or the error the visit failed with.
async fn first_visit(config: &InertiaConfig) -> Result<String, FrameworkError> {
    let response = InertiaResponse::new("Dashboard")
        .with("greeting", "hello")
        .with_config(config.clone())
        .resolve(&MockReq::new("/dashboard"))
        .await?;
    Ok(String::from_utf8_lossy(response.body()).into_owned())
}

/// Whether `document` is the client-rendered shell the Inertia client can
/// start from: the page data element and an empty mount element.
fn renders_on_the_client(document: &str) -> bool {
    document.contains("<script type=\"application/json\" data-page=\"app\">")
        && document.contains("<div id=\"app\"></div>")
        && !document.contains("data-server-rendered")
}

// ---- PAR-057: SSR on by default, gated by bundle detection ----

#[test]
fn inssr_ssr_is_on_by_default_at_laravels_worker_address() {
    let config = SsrConfig::default();
    assert!(config.enabled, "SSR is on by default, as Laravel's");
    assert_eq!(config.url, "http://127.0.0.1:13714");
    assert!(config.ensure_bundle_exists);
    assert!(InertiaConfig::new().ssr.enabled);
}

#[tokio::test]
async fn inssr_without_a_bundle_a_first_visit_renders_on_the_client_with_no_request() {
    // `framework/` holds no bundle at any conventional path, and none is
    // configured, so the worker must not be asked and nothing reported.
    let worker = Worker::rendering().await;
    let (config, errors) = with_error_hook(InertiaConfig::new().production().ssr(worker.url()));

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    assert!(
        worker.seen().is_empty(),
        "the worker was asked: {:?}",
        worker.seen()
    );
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );
}

#[tokio::test]
async fn inssr_a_configured_bundle_that_is_missing_is_skipped_quietly() {
    let worker = Worker::rendering().await;
    let (config, errors) = with_error_hook(
        InertiaConfig::new()
            .production()
            .ssr(worker.url())
            .ssr_bundle_path("/nonexistent/suprnova-ssr/ssr.js"),
    );

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    assert!(worker.seen().is_empty());
    assert!(
        errors.lock().unwrap().is_empty(),
        "a missing bundle is not a worker failure: {:?}",
        errors.lock().unwrap()
    );
}

/// The child of [`inssr_a_missing_bundle_prints_nothing`]: two first
/// visits with no error hook, one with a configured bundle that is
/// missing and one with none configured, both at an address nothing
/// listens on.
#[tokio::test]
async fn inssr_a_missing_bundle_prints_nothing_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let unreachable = closed_url().await;
    let configured = InertiaConfig::new()
        .production()
        .ssr(unreachable.clone())
        .ssr_bundle_path("/nonexistent/suprnova-ssr/ssr.js");
    let detected = InertiaConfig::new().production().ssr(unreachable);
    for config in [configured, detected] {
        let document = first_visit(&config).await.expect("the visit renders");
        assert!(renders_on_the_client(&document), "{document}");
    }
}

#[tokio::test]
async fn inssr_a_missing_bundle_prints_nothing() {
    // With SSR on by default, a message per first visit would reach the
    // log of every application that has no bundle; the child's stderr
    // shows whether one was written.
    let child = {
        let _env = crate::env_lock::lock_env_async().await;
        crate::own_process::child_command(
            "ssr_gateway::inssr_a_missing_bundle_prints_nothing_child",
        )
        .spawn()
        .expect("spawn the child process")
    };
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .expect("the wait did not panic")
        .expect("wait for the child process");
    crate::own_process::assert_child_passed(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("[inertia]"),
        "the child printed:\n{stderr}"
    );
}

#[tokio::test]
async fn inssr_a_bundle_at_a_conventional_path_dispatches_the_visit() {
    if crate::own_process_async::delegate(
        module_path!(),
        "inssr_a_bundle_at_a_conventional_path_dispatches_the_visit",
    )
    .await
    {
        return;
    }
    // This process is the test's own, so it can move to a directory that
    // holds a bundle where Laravel's detector looks for one.
    let root = tempfile::tempdir().unwrap();
    let bundle = root.path().join("frontend/bootstrap/ssr/ssr.mjs");
    std::fs::create_dir_all(bundle.parent().unwrap()).unwrap();
    std::fs::write(&bundle, b"export default {}").unwrap();
    std::env::set_current_dir(root.path()).unwrap();

    let worker = Worker::rendering().await;
    // The default configuration with only the worker's address changed.
    let mut config = InertiaConfig::new().production();
    config.ssr.url = worker.url();

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    let seen = worker.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("POST", "/render")
    );
}

#[tokio::test]
async fn inssr_a_configured_bundle_path_that_exists_dispatches_the_visit() {
    let root = tempfile::tempdir().unwrap();
    let bundle = root.path().join("ssr.js");
    std::fs::write(&bundle, b"export default {}").unwrap();
    let worker = Worker::rendering().await;
    let config = InertiaConfig::new()
        .production()
        .ssr(worker.url())
        .ssr_bundle_path(&bundle);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    assert_eq!(worker.seen().len(), 1);
}

#[tokio::test]
async fn inssr_ensure_bundle_exists_off_dispatches_without_a_bundle() {
    let worker = Worker::rendering().await;
    let mut config = InertiaConfig::new()
        .production()
        .ssr_ensure_bundle_exists(false);
    config.ssr.url = worker.url();

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    let seen = worker.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/render");
    let page: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(page["component"], "Dashboard");
}
