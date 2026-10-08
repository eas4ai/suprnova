//! `ssr:start`, `ssr:stop` and `ssr:check` (PAR-061): the shared
//! implementation in `suprnova::console::ssr`.
//!
//! A fake worker is a TCP listener the test drives: `/shutdown` closes the
//! connection without an answer, answers 200, or holds the connection open,
//! and `/health` answers with a status. A fake runtime is a shell script
//! written to a temporary directory and run as `runtime bundle`, as the
//! command runs `node ssr.js`.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use suprnova::console::ssr;
use suprnova::testing::TestContainer;
use suprnova::{FrameworkError, InertiaRequestExt, SsrConfig, SsrGateway, SsrResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_test::traced_test;

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

/// What the fake worker does with `GET /shutdown`.
#[derive(Clone, Copy)]
enum OnShutdown {
    /// Close the connection without an answer, as the Inertia SSR server
    /// does when it exits.
    Close,
    /// Answer 200 and keep running: a worker that did not stop.
    Answer,
    /// Hold the connection open and never answer.
    Hang,
}

/// One request the fake worker received.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    /// Whether the marker file existed when the request arrived: the fake
    /// runtime creates it, so `false` means no worker had started yet.
    marker_existed: bool,
}

/// A fake SSR worker on a loopback port.
struct FakeWorker {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeWorker {
    async fn start(on_shutdown: OnShutdown, health: u16, marker: Option<PathBuf>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the fake worker");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        tokio::spawn(async move {
            // Connections the worker holds open stay here until the test ends.
            let mut held = Vec::new();
            while let Ok((mut stream, _)) = listener.accept().await {
                let path = read_request_path(&mut stream).await;
                record.lock().unwrap().push(Seen {
                    path: path.clone(),
                    marker_existed: marker.as_ref().is_some_and(|m| m.exists()),
                });
                match (path.as_str(), on_shutdown) {
                    ("/shutdown", OnShutdown::Close) => drop(stream),
                    ("/shutdown", OnShutdown::Hang) => held.push(stream),
                    ("/shutdown", OnShutdown::Answer) => answer(&mut stream, 200).await,
                    ("/health", _) => answer(&mut stream, health).await,
                    _ => answer(&mut stream, 404).await,
                }
            }
        });
        Self { url, seen }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn paths(&self) -> Vec<String> {
        self.seen().into_iter().map(|s| s.path).collect()
    }
}

/// Read a request's head and return its path. The whole head is read, so
/// closing the connection afterwards is a clean close, not a reset.
async fn read_request_path(stream: &mut tokio::net::TcpStream) -> String {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&head)
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned()
}

async fn answer(stream: &mut tokio::net::TcpStream, status: u16) {
    let response =
        format!("HTTP/1.1 {status} Fake\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// A loopback URL nothing listens on, so a connection is refused at once.
///
/// A privileged port, because no test can bind one: an ephemeral port freed
/// by another test can be handed to a listener in a sibling test.
fn refusing_url() -> String {
    for port in 1..1024u16 {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
            return format!("http://{addr}");
        }
    }
    panic!("every privileged loopback port accepted a connection");
}

/// Write an executable `#!/bin/sh` script.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make the script executable");
    path
}

/// Write a bundle file the fake runtime is handed.
fn bundle(dir: &Path) -> PathBuf {
    let path = dir.join("ssr.js");
    std::fs::write(&path, "// the SSR bundle\n").expect("write the bundle");
    path
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("a UTF-8 temporary path").to_owned()
}

/// An enabled configuration for a worker at `url` with this bundle and runtime.
fn config(url: &str, bundle: Option<&Path>, runtime: &Path) -> SsrConfig {
    SsrConfig {
        enabled: true,
        url: url.to_owned(),
        timeout: Duration::from_secs(5),
        bundle_path: bundle.map(Path::to_path_buf),
        runtime: path_str(runtime),
        ..SsrConfig::default()
    }
}

/// What a command returned and wrote.
struct Ran {
    code: i32,
    out: String,
    err: String,
}

async fn start(config: &SsrConfig, runtime: Option<String>) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = ssr::start(config, runtime, &mut out, &mut err)
        .await
        .expect("ssr::start");
    ran(code, out, err)
}

async fn stop(config: &SsrConfig, graceful: bool) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = ssr::stop(config, graceful, &mut out, &mut err)
        .await
        .expect("ssr::stop");
    ran(code, out, err)
}

async fn check(config: &SsrConfig) -> Ran {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = ssr::check(config, &mut out, &mut err)
        .await
        .expect("ssr::check");
    ran(code, out, err)
}

fn ran(code: i32, out: Vec<u8>, err: Vec<u8>) -> Ran {
    Ran {
        code,
        out: String::from_utf8(out).expect("UTF-8 output"),
        err: String::from_utf8(err).expect("UTF-8 errors"),
    }
}

/// A gateway with no health check: it overrides nothing but `dispatch`.
struct NoHealthCheck;

#[suprnova::async_trait]
impl SsrGateway for NoHealthCheck {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        _page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        Ok(None)
    }
}

/// A gateway whose health check answers `healthy`, and records the worker
/// URL it was asked about.
struct Health {
    healthy: bool,
    asked: Mutex<Vec<String>>,
}

impl Health {
    fn new(healthy: bool) -> Self {
        Self {
            healthy,
            asked: Mutex::new(Vec::new()),
        }
    }
}

#[suprnova::async_trait]
impl SsrGateway for Health {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        _page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        Ok(None)
    }

    async fn is_healthy(&self, config: &SsrConfig) -> Option<bool> {
        self.asked.lock().unwrap().push(config.url.clone());
        Some(self.healthy)
    }
}

// ---------------------------------------------------------------------------
// The shared implementation: ssr:start
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inssr_start_refuses_when_ssr_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'", marker.display()),
    );
    let bundle = bundle(dir.path());
    let config = SsrConfig {
        enabled: false,
        ..config(&refusing_url(), Some(&bundle), &runtime)
    };

    let ran = start(&config, None).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err.contains("Inertia SSR is not enabled."),
        "the refusal names the reason: {}",
        ran.err
    );
    assert!(
        ran.err.contains("InertiaConfig::ssr"),
        "and says how to enable it: {}",
        ran.err
    );
    assert!(!marker.exists(), "no worker started with SSR disabled");
}

#[tokio::test]
async fn inssr_start_fails_naming_the_configured_bundle_when_none_exists() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'", marker.display()),
    );
    let missing = dir.path().join("missing-ssr.js");

    let ran = start(&config(&refusing_url(), Some(&missing), &runtime), None).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err.contains(&format!(
            "Inertia SSR bundle not found at the configured path: \"{}\"",
            missing.display()
        )),
        "the failure names the configured path: {}",
        ran.err
    );
    assert!(!marker.exists(), "no worker started without a bundle");
}

#[tokio::test]
async fn inssr_start_says_how_to_set_the_bundle_when_none_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'", marker.display()),
    );

    // The test runs in the framework's directory, which holds none of the
    // conventional bundle paths.
    let ran = start(&config(&refusing_url(), None, &runtime), None).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err.contains("Inertia SSR bundle not found."),
        "the failure says no bundle was found: {}",
        ran.err
    );
    assert!(
        ran.err.contains("InertiaConfig::ssr_bundle_path")
            && ran.err.contains("SUPRNOVA_SSR_BUNDLE"),
        "and how to set one: {}",
        ran.err
    );
    assert!(!marker.exists(), "no worker started without a bundle");
}

#[tokio::test]
async fn inssr_start_refuses_a_runtime_it_cannot_find_when_ensure_runtime_exists() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let not_a_file = dir.path().join("no-such-dir").join("node");
    for runtime in [
        path_str(&not_a_file),
        "suprnova-inssr-no-such-runtime".to_owned(),
    ] {
        let config = SsrConfig {
            ensure_runtime_exists: true,
            runtime: runtime.clone(),
            ..config(&refusing_url(), Some(&bundle), &not_a_file)
        };

        let ran = start(&config, None).await;

        assert_eq!(ran.code, 1, "{runtime}: out: {} err: {}", ran.out, ran.err);
        assert!(
            ran.err
                .contains(&format!("SSR runtime \"{runtime}\" could not be found.")),
            "the refusal names the runtime: {}",
            ran.err
        );
    }
}

#[tokio::test]
async fn inssr_start_runs_a_runtime_it_finds_when_ensure_runtime_exists() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let runtime = script(dir.path(), "runtime", "echo \"ran $1\"");
    let config = SsrConfig {
        ensure_runtime_exists: true,
        ..config(&refusing_url(), Some(&bundle), &runtime)
    };

    let ran = start(&config, None).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.out.contains(&format!("ran {}", bundle.display())),
        "the runtime ran with the bundle: {}",
        ran.out
    );
}

#[tokio::test]
async fn inssr_start_stops_a_running_worker_before_it_starts_one() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let worker = FakeWorker::start(OnShutdown::Close, 200, Some(marker.clone())).await;
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'\necho started", marker.display()),
    );
    let bundle = bundle(dir.path());

    let ran = start(&config(&worker.url, Some(&bundle), &runtime), None).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    let seen = worker.seen();
    assert_eq!(
        worker.paths(),
        vec!["/shutdown".to_owned()],
        "the running worker was asked to shut down"
    );
    assert!(
        !seen[0].marker_existed,
        "the shutdown came before the new worker started"
    );
    assert!(marker.exists(), "the new worker started after it");
    assert!(
        !ran.out.contains("Inertia SSR server stopped."),
        "the stop runs silently, as Laravel's callSilently: {}",
        ran.out
    );
}

#[tokio::test]
async fn inssr_start_with_a_runtime_override_runs_that_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let configured = script(dir.path(), "configured", "echo configured");
    let flagged = script(dir.path(), "flagged", "echo flagged");

    let ran = start(
        &config(&refusing_url(), Some(&bundle), &configured),
        Some(path_str(&flagged)),
    )
    .await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(ran.out.contains("flagged"), "{}", ran.out);
    assert!(!ran.out.contains("configured"), "{}", ran.out);
}

#[tokio::test]
#[traced_test]
async fn inssr_start_forwards_stdout_reports_stderr_and_exits_with_the_workers_code() {
    let dir = tempfile::tempdir().unwrap();
    let bundle = bundle(dir.path());
    let runtime = script(
        dir.path(),
        "runtime",
        "echo \"rendering $1\"\necho 'first problem' >&2\necho 'second problem' >&2\nexit 3",
    );

    let ran = start(&config(&refusing_url(), Some(&bundle), &runtime), None).await;

    assert_eq!(
        ran.code, 3,
        "the worker's exit status: {} {}",
        ran.out, ran.err
    );
    assert!(
        ran.out.contains(&format!("rendering {}", bundle.display())),
        "stdout is forwarded as output: {}",
        ran.out
    );
    assert!(
        !ran.out.contains("problem"),
        "stderr is not output: {}",
        ran.out
    );
    assert!(
        ran.err.contains("first problem\n") && ran.err.contains("second problem\n"),
        "every stderr line is reported on the error stream: {}",
        ran.err
    );
    logs_assert(|lines: &[&str]| {
        for problem in ["first problem", "second problem"] {
            if !lines
                .iter()
                .any(|line| line.contains("ERROR") && line.contains(problem))
            {
                return Err(format!("no error event for {problem:?}: {lines:#?}"));
            }
        }
        Ok(())
    });
}

// ---------------------------------------------------------------------------
// The shared implementation: ssr:stop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inssr_stop_succeeds_when_the_worker_closes_the_connection() {
    let worker = FakeWorker::start(OnShutdown::Close, 200, None).await;
    let config = SsrConfig {
        url: worker.url.clone(),
        ..SsrConfig::default()
    };

    let ran = stop(&config, false).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.out, "Inertia SSR server stopped.\n");
    assert_eq!(worker.paths(), vec!["/shutdown".to_owned()]);
}

#[tokio::test]
async fn inssr_stop_fails_when_the_worker_answers_and_keeps_running() {
    let worker = FakeWorker::start(OnShutdown::Answer, 200, None).await;
    let config = SsrConfig {
        url: worker.url.clone(),
        ..SsrConfig::default()
    };

    for graceful in [false, true] {
        let ran = stop(&config, graceful).await;

        assert_eq!(ran.code, 1, "graceful {graceful}: {} {}", ran.out, ran.err);
        assert_eq!(ran.err, "Unable to connect to Inertia SSR server.\n");
    }
}

#[tokio::test]
async fn inssr_stop_without_graceful_fails_when_no_worker_runs() {
    let config = SsrConfig {
        url: refusing_url(),
        ..SsrConfig::default()
    };

    let ran = stop(&config, false).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.err, "Unable to connect to Inertia SSR server.\n");
}

#[tokio::test]
async fn inssr_stop_with_graceful_succeeds_when_no_worker_runs() {
    let config = SsrConfig {
        url: refusing_url(),
        ..SsrConfig::default()
    };

    let ran = stop(&config, true).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.out, "Inertia SSR server is not running.\n");
}

#[tokio::test]
async fn inssr_stop_gives_up_on_a_worker_that_never_answers() {
    let worker = FakeWorker::start(OnShutdown::Hang, 200, None).await;
    let config = SsrConfig {
        url: worker.url.clone(),
        timeout: Duration::from_millis(200),
        ..SsrConfig::default()
    };

    for graceful in [false, true] {
        let ran = stop(&config, graceful).await;

        assert_eq!(ran.code, 1, "graceful {graceful}: {} {}", ran.out, ran.err);
        assert_eq!(ran.err, "Unable to connect to Inertia SSR server.\n");
    }
}

#[tokio::test]
async fn inssr_stop_and_check_refuse_a_url_without_a_scheme() {
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn SsrGateway>(Arc::new(Health::new(true)));
    let config = SsrConfig {
        url: "127.0.0.1:13714".to_owned(),
        ..SsrConfig::default()
    };

    for ran in [stop(&config, true).await, check(&config).await] {
        assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
        assert!(
            ran.err.contains("http://") && ran.err.contains("127.0.0.1:13714"),
            "the error names the URL and the scheme it needs: {}",
            ran.err
        );
    }
}

// ---------------------------------------------------------------------------
// The shared implementation: ssr:check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inssr_check_fails_when_the_gateway_has_no_health_check() {
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn SsrGateway>(Arc::new(NoHealthCheck));

    let ran = check(&SsrConfig::default()).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.err, "The SSR gateway does not support health checks.\n");
}

#[tokio::test]
async fn inssr_check_fails_when_the_worker_is_unhealthy() {
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn SsrGateway>(Arc::new(Health::new(false)));

    let ran = check(&SsrConfig::default()).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.err, "Inertia SSR server is not running.\n");
}

#[tokio::test]
async fn inssr_check_succeeds_when_the_worker_is_healthy() {
    let _container = TestContainer::fake();
    let gateway = Arc::new(Health::new(true));
    TestContainer::bind::<dyn SsrGateway>(gateway.clone());
    let config = SsrConfig {
        url: "http://ssr.internal:4000".to_owned(),
        ..SsrConfig::default()
    };

    let ran = check(&config).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(ran.out, "Inertia SSR server is running.\n");
    assert_eq!(
        *gateway.asked.lock().unwrap(),
        vec!["http://ssr.internal:4000".to_owned()],
        "the gateway was asked about the configured worker"
    );
}
