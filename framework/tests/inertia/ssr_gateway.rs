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
use suprnova::testing::TestContainer;
use suprnova::{
    App, EventFacade, FrameworkError, Inertia, InertiaConfig, InertiaRequestExt, InertiaResponse,
    SsrCondition, SsrConfig, SsrErrorType, SsrGateway, SsrRenderFailed, SsrRequestConfigurator,
    SsrResponse,
};

use crate::protocol_harness::MockReq;

/// The body a worker that renders answers with.
const RENDERED: &str = "<div data-server-rendered=\"true\" id=\"app\">rendered</div>";

/// One request a fake worker received.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: hyper::HeaderMap,
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
                                headers: parts.headers,
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

// ---- PAR-058: hot mode through the Vite dev server ----

#[tokio::test]
async fn inssr_development_dispatches_to_the_dev_server_without_a_bundle() {
    // No bundle on disk and the bundle check on: hot mode skips it and
    // posts to the dev server's `/__inertia_ssr`, never to the worker.
    let dev_server = Worker::rendering().await;
    let worker = Worker::rendering().await;
    let config = InertiaConfig::new()
        .development(true)
        .vite_dev_server(dev_server.url())
        .ssr(worker.url());

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    let seen = dev_server.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("POST", "/__inertia_ssr")
    );
    let page: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(page["component"], "Dashboard");
    assert!(
        worker.seen().is_empty(),
        "the worker was asked: {:?}",
        worker.seen()
    );
}

#[tokio::test]
async fn inssr_a_configured_hot_url_is_the_address_used() {
    let hot = Worker::rendering().await;
    let dev_server = Worker::rendering().await;
    let worker = Worker::rendering().await;
    let config = InertiaConfig::new()
        .development(true)
        .vite_dev_server(dev_server.url())
        .ssr_hot_url(format!("{}/", hot.url()))
        .ssr(worker.url());

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    let seen = hot.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].path, "/__inertia_ssr");
    assert!(dev_server.seen().is_empty());
    assert!(worker.seen().is_empty());
}

#[tokio::test]
async fn inssr_production_posts_to_the_worker_and_never_the_hot_url() {
    let hot = Worker::rendering().await;
    let worker = Worker::rendering().await;
    let config = InertiaConfig::new()
        .production()
        .vite_dev_server(hot.url())
        .ssr_hot_url(hot.url())
        .ssr(worker.url())
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    assert!(
        hot.seen().is_empty(),
        "production went hot: {:?}",
        hot.seen()
    );
    assert_eq!(worker.seen().len(), 1);
    assert_eq!(worker.seen()[0].path, "/render");
}

#[tokio::test]
async fn inssr_development_without_a_dev_server_listening_takes_the_worker_path() {
    // Laravel goes hot only while Vite runs; with nothing listening at the
    // dev server's address the visit takes the worker path, bundle check
    // included.
    let worker = Worker::rendering().await;
    let checked = InertiaConfig::new()
        .development(true)
        .vite_dev_server(closed_url().await)
        .ssr(worker.url());
    let (checked, errors) = with_error_hook(checked);

    let document = first_visit(&checked).await.expect("the visit renders");
    assert!(renders_on_the_client(&document), "{document}");
    assert!(worker.seen().is_empty());
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );

    let unchecked = checked.ssr_ensure_bundle_exists(false);
    let document = first_visit(&unchecked).await.expect("the visit renders");
    assert!(document.contains(RENDERED), "{document}");
    assert_eq!(worker.seen().len(), 1);
    assert_eq!(worker.seen()[0].path, "/render");
}

// ---- PAR-059: https workers ----

#[tokio::test]
async fn inssr_an_https_worker_url_is_spoken_to_in_tls() {
    use tokio::io::AsyncReadExt;
    // No certificate is needed to see which protocol arrived: a TLS client
    // opens with a handshake record (content type 0x16, version 0x03 ..),
    // a plain HTTP client with the request line. The listener closes the
    // connection once it has read two bytes, which is what ends the visit.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let first_bytes = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 2];
        stream.read_exact(&mut bytes).await.unwrap();
        bytes
    });
    let config = InertiaConfig::new()
        .production()
        .ssr(format!("https://{addr}"))
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config)
        .await
        .expect("a worker failure renders on the client");

    assert!(renders_on_the_client(&document), "{document}");
    // The visit ends only after the listener read and closed, so a task
    // still waiting means no connection was ever made.
    assert!(
        first_bytes.is_finished(),
        "the client never connected to the https worker"
    );
    let bytes = first_bytes.await.unwrap();
    assert_eq!(
        bytes,
        [0x16, 0x03],
        "the worker received {:?}, not a TLS handshake",
        String::from_utf8_lossy(&bytes)
    );
}

// ---- PAR-059: an empty or falsy worker answer ----

#[tokio::test]
async fn inssr_an_empty_or_falsy_worker_answer_renders_on_the_client() {
    // Laravel's `if (! $data = $response->json()) return null;`: nothing
    // to inline means the client renders, from the page data the shell
    // carries. An answer with no body is the same: inlining it would leave
    // the document with neither the page data nor the mount element.
    let answers = [
        "{}",
        "null",
        "false",
        "[]",
        "\"<div id=\\\"app\\\"></div>\"",
        "0",
        "",
        "{\"head\": [\"<title>From the worker</title>\"]}",
        "{\"head\": [], \"body\": \"\"}",
        "{\"head\": [], \"body\": null}",
    ];
    for answer in answers {
        let worker = Worker::answering(200, answer).await;
        let (config, errors) = with_error_hook(
            InertiaConfig::new()
                .production()
                .ssr(worker.url())
                .ssr_ensure_bundle_exists(false),
        );

        let document = first_visit(&config).await.expect("the visit renders");

        assert!(
            renders_on_the_client(&document),
            "the answer {answer:?} left the document without the client shell:\n{document}"
        );
        assert!(
            !document.contains("From the worker"),
            "{answer:?}:\n{document}"
        );
        assert_eq!(worker.seen().len(), 1, "{answer:?}");
        assert!(
            errors.lock().unwrap().is_empty(),
            "an empty answer is not a failure: {answer:?} {:?}",
            errors.lock().unwrap()
        );
    }
}

#[tokio::test]
async fn inssr_a_head_entry_that_is_not_a_string_is_left_out() {
    let answer = json!({
        "head": ["<title>From the worker</title>", 42, null],
        "body": RENDERED,
    });
    let worker = Worker::answering(200, answer.to_string()).await;
    let config = InertiaConfig::new()
        .production()
        .ssr(worker.url())
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    assert!(
        document.contains("<title>From the worker</title>"),
        "{document}"
    );
}

// ---- PAR-059: the worker's error answer and SsrRenderFailed ----

/// The error JSON an Inertia 3 worker answers a failed render with.
fn browser_api_error() -> Value {
    json!({
        "error": "window is not defined",
        "type": "browser-api",
        "hint": "Move the access into onMounted()",
        "browserApi": "window",
        "stack": "ReferenceError: window is not defined\n    at setup (Dashboard.vue:12:5)",
        "sourceLocation": "resources/js/Pages/Dashboard.vue:12:5",
    })
}

#[tokio::test]
async fn inssr_a_worker_error_dispatches_ssr_render_failed_with_the_component() {
    let _events = EventFacade::fake();
    let worker = Worker::answering(500, browser_api_error().to_string()).await;
    let (config, errors) = with_error_hook(
        InertiaConfig::new()
            .production()
            .ssr(worker.url())
            .ssr_ensure_bundle_exists(false),
    );

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    let failures = suprnova::events::dispatched::<SsrRenderFailed>(|_| true);
    assert_eq!(
        failures,
        vec![SsrRenderFailed {
            component: "Dashboard".to_string(),
            url: "/dashboard".to_string(),
            error: "window is not defined".to_string(),
            error_type: SsrErrorType::BrowserApi,
            hint: Some("Move the access into onMounted()".to_string()),
            browser_api: Some("window".to_string()),
            stack: Some(
                "ReferenceError: window is not defined\n    at setup (Dashboard.vue:12:5)"
                    .to_string()
            ),
            source_location: Some("resources/js/Pages/Dashboard.vue:12:5".to_string()),
        }]
    );
    let errors = errors.lock().unwrap();
    assert_eq!(errors.len(), 1, "the on_error hook still fires: {errors:?}");
    assert!(errors[0].contains("[Dashboard]"), "{errors:?}");
    assert!(errors[0].contains("window is not defined"), "{errors:?}");
}

#[tokio::test]
async fn inssr_an_error_answer_without_details_is_an_unknown_failure() {
    let _events = EventFacade::fake();
    let worker = Worker::answering(502, "Bad Gateway").await;
    let config = InertiaConfig::new()
        .production()
        .ssr(worker.url())
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    let failures = suprnova::events::dispatched::<SsrRenderFailed>(|_| true);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error_type, SsrErrorType::Unknown);
    assert_eq!(failures[0].component, "Dashboard");
    assert!(failures[0].error.contains("502"), "{:?}", failures[0]);
    assert_eq!(failures[0].hint, None);
}

#[tokio::test]
async fn inssr_a_refused_connection_dispatches_a_connection_failure() {
    let _events = EventFacade::fake();
    let config = InertiaConfig::new()
        .production()
        .ssr(closed_url().await)
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    let failures = suprnova::events::dispatched::<SsrRenderFailed>(|_| true);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].error_type, SsrErrorType::Connection);
    assert_eq!(failures[0].component, "Dashboard");
    assert_eq!(failures[0].url, "/dashboard");
    assert!(!failures[0].error.is_empty());
}

#[tokio::test]
async fn inssr_throw_on_error_names_the_component_and_the_source_location() {
    let _events = EventFacade::fake();
    let worker = Worker::answering(500, browser_api_error().to_string()).await;
    let config = InertiaConfig::new()
        .production()
        .ssr(worker.url())
        .ssr_ensure_bundle_exists(false)
        .ssr_throw_on_error(true);

    let error = first_visit(&config)
        .await
        .expect_err("the visit fails under throw_on_error");

    assert!(
        error.to_string().contains(
            "SSR render failed for component [Dashboard]: window is not defined \
             at resources/js/Pages/Dashboard.vue:12:5"
        ),
        "{error}"
    );
    // The event is dispatched before the visit fails, as Laravel's.
    assert_eq!(
        suprnova::events::dispatched::<SsrRenderFailed>(|_| true).len(),
        1
    );

    // Without a source location the message ends at the error.
    let mut details = browser_api_error();
    details.as_object_mut().unwrap().remove("sourceLocation");
    let worker = Worker::answering(500, details.to_string()).await;
    let config = config.ssr(worker.url());
    let error = first_visit(&config).await.expect_err("the visit fails");
    assert!(
        error
            .to_string()
            .ends_with("SSR render failed for component [Dashboard]: window is not defined"),
        "{error}"
    );
}

#[tokio::test]
async fn inssr_throw_on_error_names_the_component_of_an_unreachable_worker() {
    let config = InertiaConfig::new()
        .production()
        .ssr(closed_url().await)
        .ssr_ensure_bundle_exists(false)
        .ssr_throw_on_error(true);

    let error = first_visit(&config).await.expect_err("the visit fails");

    assert!(
        error
            .to_string()
            .contains("SSR render failed for component [Dashboard]: "),
        "{error}"
    );
}

#[test]
fn inssr_ssr_error_types_carry_laravels_names() {
    for (name, kind) in [
        ("browser-api", SsrErrorType::BrowserApi),
        ("component-resolution", SsrErrorType::ComponentResolution),
        ("render", SsrErrorType::Render),
        ("connection", SsrErrorType::Connection),
        ("unknown", SsrErrorType::Unknown),
    ] {
        assert_eq!(SsrErrorType::from_name(name), kind);
        assert_eq!(kind.as_str(), name);
        assert_eq!(kind.to_string(), name);
    }
    assert_eq!(SsrErrorType::from_name("timeout"), SsrErrorType::Unknown);
}

// ---- PAR-060: the gateway as a binding the facade acts on ----

/// What a [`RecordingGateway`] was asked.
#[derive(Default)]
struct Calls {
    dispatched: Vec<String>,
    disabled: Option<bool>,
    excepted: Vec<String>,
    configured: bool,
}

/// An application's own gateway: it renders every page itself, implements
/// every capability, and records the calls.
#[derive(Default)]
struct RecordingGateway {
    calls: Mutex<Calls>,
}

#[suprnova::async_trait]
impl SsrGateway for RecordingGateway {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        let mut calls = self.calls.lock().unwrap();
        calls
            .dispatched
            .push(page["component"].as_str().unwrap_or_default().to_string());
        if calls.disabled == Some(true) {
            return Ok(None);
        }
        Ok(Some(SsrResponse {
            head: vec!["<meta name=\"gateway\" content=\"own\">".to_string()],
            body: "<div data-server-rendered=\"true\" id=\"app\">from the gateway</div>"
                .to_string(),
        }))
    }

    fn disable(&self, condition: SsrCondition) -> bool {
        let disabled = match condition {
            SsrCondition::Always(disabled) => disabled,
            SsrCondition::When(_) => false,
        };
        self.calls.lock().unwrap().disabled = Some(disabled);
        true
    }

    fn except(&self, paths: Vec<String>) -> bool {
        self.calls.lock().unwrap().excepted.extend(paths);
        true
    }

    fn configure_request_using(&self, _configure: SsrRequestConfigurator) -> bool {
        self.calls.lock().unwrap().configured = true;
        true
    }
}

/// A gateway with no capabilities and no health check.
struct BareGateway;

#[suprnova::async_trait]
impl SsrGateway for BareGateway {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        _page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        Ok(None)
    }
}

#[tokio::test]
async fn inssr_a_bound_gateway_is_the_one_a_first_visit_dispatches_through() {
    let _container = TestContainer::fake();
    let gateway = Arc::new(RecordingGateway::default());
    App::bind::<dyn SsrGateway>(gateway.clone());
    // No bundle and a worker nobody runs: only the bound gateway renders.
    let config = InertiaConfig::new().production();

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(
        document.contains("<meta name=\"gateway\" content=\"own\">"),
        "{document}"
    );
    assert!(document.contains("from the gateway"), "{document}");
    assert_eq!(gateway.calls.lock().unwrap().dispatched, ["Dashboard"]);
}

#[tokio::test]
async fn inssr_the_facade_acts_on_the_bound_gateway() {
    let _container = TestContainer::fake();
    let gateway = Arc::new(RecordingGateway::default());
    App::bind::<dyn SsrGateway>(gateway.clone());
    let config = InertiaConfig::new().production();

    Inertia::disable_ssr(true);
    Inertia::without_ssr(["admin/*"]);
    Inertia::configure_ssr_request_using(|request| request.header("X-Token", "t"));
    let document = first_visit(&config).await.expect("the visit renders");

    assert!(renders_on_the_client(&document), "{document}");
    let calls = gateway.calls.lock().unwrap();
    assert_eq!(calls.disabled, Some(true));
    assert_eq!(calls.excepted, ["admin/*"]);
    assert!(calls.configured);
    assert_eq!(
        calls.dispatched,
        ["Dashboard"],
        "dispatched through the gateway"
    );
}

#[tokio::test]
#[tracing_test::traced_test]
async fn inssr_a_capability_the_bound_gateway_lacks_is_logged() {
    let _container = TestContainer::fake();
    App::bind::<dyn SsrGateway>(Arc::new(BareGateway));

    Inertia::disable_ssr(true);
    Inertia::disable_ssr_if(|_: &dyn InertiaRequestExt| true);
    Inertia::without_ssr(["admin/*"]);
    Inertia::configure_ssr_request_using(|request| request);

    assert!(logs_contain("cannot disable SSR"));
    assert!(logs_contain("cannot exclude paths from SSR"));
    assert!(logs_contain("cannot configure the SSR request"));
    assert_eq!(Inertia::ssr_is_healthy().await, None);
}

#[tokio::test]
async fn inssr_with_nothing_bound_the_http_gateway_posts_to_the_worker() {
    let _container = TestContainer::fake();
    let worker = Worker::rendering().await;
    let config = InertiaConfig::new()
        .production()
        .ssr(worker.url())
        .ssr_ensure_bundle_exists(false);

    let document = first_visit(&config).await.expect("the visit renders");

    assert!(document.contains(RENDERED), "{document}");
    assert_eq!(worker.seen().len(), 1);
    assert_eq!(worker.seen()[0].path, "/render");
}

/// The installed configuration with the worker at `url`, under the test's
/// own container.
fn install_worker(url: String) {
    Inertia::install(
        &InertiaConfig::new()
            .development(true)
            .register_globally(false)
            .ssr(url),
    )
    .expect("install");
}

#[tokio::test]
async fn inssr_ssr_is_healthy_reads_the_workers_health_route() {
    let _container = TestContainer::fake();
    let healthy = Worker::answering(200, "{\"status\":\"OK\"}").await;
    install_worker(healthy.url());
    Inertia::configure_ssr_request_using(|request| request.header("X-Health-Token", "h"));

    assert_eq!(Inertia::ssr_is_healthy().await, Some(true));
    let seen = healthy.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].method.as_str(), seen[0].path.as_str()),
        ("GET", "/health")
    );
    assert_eq!(
        seen[0]
            .headers
            .get("x-health-token")
            .and_then(|v| v.to_str().ok()),
        Some("h"),
        "the request configurator applies to the health check"
    );

    let failing = Worker::answering(500, "{}").await;
    install_worker(failing.url());
    assert_eq!(Inertia::ssr_is_healthy().await, Some(false));

    install_worker(closed_url().await);
    assert_eq!(Inertia::ssr_is_healthy().await, Some(false));
}
