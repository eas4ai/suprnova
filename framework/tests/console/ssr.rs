//! `ssr:start`, `ssr:stop` and `ssr:check` (PAR-061): the shared
//! implementation in `suprnova::console::ssr`, and the application binary's
//! commands, which run it with the Inertia configuration the application
//! installed.
//!
//! A fake worker is a TCP listener the test drives: `/shutdown` closes the
//! connection without an answer, answers 200, or holds the connection open,
//! and `/health` answers with a status. A fake runtime is a shell script
//! written to a temporary directory and run as `runtime bundle`, as the
//! command runs `node ssr.js`.
//!
//! The application binary's commands run in a child process: the child is
//! this test binary running [`ssr_app_child`], which builds an `Application`
//! from its environment and runs one subcommand with `run_with_args`. A child
//! has a stdout, a stderr, an exit status and a process id of its own, so the
//! tests read the command's output and send it signals without touching the
//! process the test runs in.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use suprnova::console::ssr;
use suprnova::testing::TestContainer;
use suprnova::{
    App, FrameworkError, Inertia, InertiaConfig, InertiaRequestExt, SsrConfig, SsrGateway,
    SsrResponse,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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

/// A gateway whose health check answers `healthy`, and records (and prints,
/// for a child process's parent) the worker URL it was asked about.
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
        println!("gateway asked about {}", config.url);
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
        ran.err.contains("InertiaConfig::ssr_bundle_path"),
        "and how to set one: {}",
        ran.err
    );
    for path in suprnova::CONVENTIONAL_BUNDLE_PATHS {
        assert!(
            ran.err.contains(path),
            "and the conventional path {path} it was looked for at: {}",
            ran.err
        );
    }
    assert!(
        !ran.err.contains("--bundle") && !ran.err.contains("SUPRNOVA_SSR_BUNDLE"),
        "the CLI has no bundle flag or variable of its own: {}",
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
async fn inssr_start_refuses_when_the_running_worker_does_not_stop() {
    // A worker that answers the shutdown request and keeps running has not
    // stopped: PAR-061 says the running worker stops first, so no second
    // worker starts beside it (it could not bind the port in any case).
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let worker = FakeWorker::start(OnShutdown::Answer, 200, Some(marker.clone())).await;
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'\necho started", marker.display()),
    );
    let bundle = bundle(dir.path());

    let ran = start(&config(&worker.url, Some(&bundle), &runtime), None).await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert_eq!(
        worker.paths(),
        vec!["/shutdown".to_owned()],
        "the running worker was asked to shut down"
    );
    assert!(!marker.exists(), "no second worker started: {}", ran.out);
    assert!(
        ran.err.contains("did not stop") && ran.err.contains(&worker.url),
        "the refusal names the worker that is still running: {}",
        ran.err
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

// ---------------------------------------------------------------------------
// The application binary's commands
// ---------------------------------------------------------------------------

/// What the child prints before a failure `run_with_args` returned to it.
const RETURNED: &str = "run_with_args returned: ";

/// The child half of every application-binary test: an `Application` built
/// from the `INSSR_CHILD_*` environment runs `INSSR_CHILD_ARGS`. It does
/// nothing unless a parent started it.
///
/// - `INSSR_CHILD_INSTALL`: `http` installs Inertia in the HTTP hook, as the
///   scaffold does, `bootstrap` in the process-wide hook, `none` not at all.
/// - `INSSR_CHILD_SSR`: the worker URL, or `disabled`.
/// - `INSSR_CHILD_BUNDLE`, `INSSR_CHILD_RUNTIME`: the bundle path and the
///   runtime; `INSSR_CHILD_ENSURE_RUNTIME=1` sets `ensure_runtime_exists`.
/// - `INSSR_CHILD_GATEWAY`: `healthy`, `unhealthy` or `none` binds that
///   gateway in the process-wide hook.
#[test]
fn ssr_app_child() {
    let Ok(args) = std::env::var("INSSR_CHILD_ARGS") else {
        return;
    };
    suprnova::boot::load_env().expect("load the configuration");
    let var = |name: &str| std::env::var(name).ok();

    let mut inertia = InertiaConfig::new();
    match var("INSSR_CHILD_SSR").as_deref() {
        Some("disabled") | None => inertia = inertia.ssr_disabled(),
        Some(url) => inertia = inertia.ssr(url),
    }
    if let Some(bundle) = var("INSSR_CHILD_BUNDLE") {
        inertia = inertia.ssr_bundle_path(bundle);
    }
    if let Some(runtime) = var("INSSR_CHILD_RUNTIME") {
        inertia = inertia.ssr_runtime(runtime);
    }
    if var("INSSR_CHILD_ENSURE_RUNTIME").as_deref() == Some("1") {
        inertia = inertia.ssr_ensure_runtime_exists(true);
    }
    let install = var("INSSR_CHILD_INSTALL").unwrap_or_else(|| "http".to_owned());
    let gateway = var("INSSR_CHILD_GATEWAY");

    let in_bootstrap = (install == "bootstrap").then(|| inertia.clone());
    let in_http = (install == "http").then_some(inertia);
    let argv: Vec<String> = std::iter::once("app".to_owned())
        .chain(args.split_whitespace().map(str::to_owned))
        .collect();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = runtime.block_on(async move {
        suprnova::Application::new()
            .bootstrap(move || async move {
                match gateway.as_deref() {
                    Some("healthy") => App::bind::<dyn SsrGateway>(Arc::new(Health::new(true))),
                    Some("unhealthy") => App::bind::<dyn SsrGateway>(Arc::new(Health::new(false))),
                    Some("none") => App::bind::<dyn SsrGateway>(Arc::new(NoHealthCheck)),
                    _ => {}
                }
                if let Some(config) = in_bootstrap {
                    Inertia::install(&config).expect("install Inertia");
                }
            })
            .http_bootstrap(move || async move {
                if let Some(config) = in_http {
                    Inertia::install(&config).expect("install Inertia");
                }
            })
            .run_with_args(argv)
            .await
    });
    // The executable boundary: the failure came back to this caller, which
    // prints it and exits non-zero, as `Application::run` does.
    if let Err(e) = outcome {
        if e.is_silent() {
            eprintln!("{RETURNED}(silent)");
        } else {
            eprintln!("{RETURNED}{}", e.message());
        }
        std::process::exit(1);
    }
}

/// A child process running the application binary's `args`.
fn app(args: &str, env: &[(&str, &str)]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "ssr::ssr_app_child", "--nocapture"])
        .env("INSSR_CHILD_ARGS", args)
        .env("APP_ENV", "testing")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    for (name, value) in env {
        command.env(name, value);
    }
    command
}

/// Run the child to the end.
async fn run_app(args: &str, env: &[(&str, &str)]) -> Ran {
    finish(&mut app(args, env)).await
}

/// Run `command` to the end and read what it printed.
async fn finish(command: &mut tokio::process::Command) -> Ran {
    let output = command.output().await.expect("run the child");
    Ran {
        code: output.status.code().expect("the child exited"),
        out: String::from_utf8_lossy(&output.stdout).into_owned(),
        err: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[tokio::test]
async fn inssr_app_check_reads_the_installed_configuration() {
    // Installed in the HTTP hook, where the scaffold installs Inertia.
    let ran = run_app(
        "ssr:check",
        &[
            ("INSSR_CHILD_SSR", "http://ssr.internal:4000"),
            ("INSSR_CHILD_GATEWAY", "healthy"),
        ],
    )
    .await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.out
            .contains("gateway asked about http://ssr.internal:4000"),
        "the gateway was asked about the installed worker: {}",
        ran.out
    );
    assert!(
        ran.out.contains("Inertia SSR server is running."),
        "{}",
        ran.out
    );
}

#[tokio::test]
async fn inssr_app_check_fails_when_the_installed_worker_is_unhealthy() {
    for (gateway, message) in [
        ("unhealthy", "Inertia SSR server is not running."),
        ("none", "The SSR gateway does not support health checks."),
    ] {
        let ran = run_app(
            "ssr:check",
            &[
                ("INSSR_CHILD_SSR", "http://127.0.0.1:13714"),
                ("INSSR_CHILD_INSTALL", "bootstrap"),
                ("INSSR_CHILD_GATEWAY", gateway),
            ],
        )
        .await;

        assert_eq!(ran.code, 1, "{gateway}: out: {} err: {}", ran.out, ran.err);
        assert!(ran.err.contains(message), "{gateway}: {}", ran.err);
    }
}

#[tokio::test]
async fn inssr_app_ssr_commands_need_an_installed_inertia_configuration() {
    for args in ["ssr:start", "ssr:stop", "ssr:check"] {
        let ran = run_app(args, &[("INSSR_CHILD_INSTALL", "none")]).await;

        assert_eq!(ran.code, 1, "{args}: out: {} err: {}", ran.out, ran.err);
        assert!(
            ran.err.contains(RETURNED) && ran.err.contains("Inertia::install"),
            "{args}: the failure says what is missing: {}",
            ran.err
        );
    }
}

#[tokio::test]
async fn inssr_app_stop_calls_the_installed_workers_shutdown() {
    let worker = FakeWorker::start(OnShutdown::Close, 200, None).await;

    let ran = run_app("ssr:stop", &[("INSSR_CHILD_SSR", &worker.url)]).await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.out.contains("Inertia SSR server stopped."),
        "{}",
        ran.out
    );
    assert_eq!(worker.paths(), vec!["/shutdown".to_owned()]);
}

#[tokio::test]
async fn inssr_app_stop_with_graceful_succeeds_when_no_worker_runs() {
    let url = refusing_url();

    let graceful = run_app("ssr:stop --graceful", &[("INSSR_CHILD_SSR", &url)]).await;
    assert_eq!(
        graceful.code, 0,
        "out: {} err: {}",
        graceful.out, graceful.err
    );
    assert!(
        graceful.out.contains("Inertia SSR server is not running."),
        "{}",
        graceful.out
    );

    let plain = run_app("ssr:stop", &[("INSSR_CHILD_SSR", &url)]).await;
    assert_eq!(plain.code, 1, "out: {} err: {}", plain.out, plain.err);
    assert!(
        plain
            .err
            .contains("Unable to connect to Inertia SSR server."),
        "{}",
        plain.err
    );
}

#[tokio::test]
async fn inssr_app_start_refuses_when_ssr_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let runtime = script(
        dir.path(),
        "runtime",
        &format!("touch '{}'", marker.display()),
    );
    let bundle = bundle(dir.path());

    let ran = run_app(
        "ssr:start",
        &[
            ("INSSR_CHILD_SSR", "disabled"),
            ("INSSR_CHILD_BUNDLE", &path_str(&bundle)),
            ("INSSR_CHILD_RUNTIME", &path_str(&runtime)),
        ],
    )
    .await;

    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err.contains("Inertia SSR is not enabled."),
        "{}",
        ran.err
    );
    assert!(!marker.exists(), "no worker started with SSR disabled");
}

#[tokio::test]
async fn inssr_app_start_runs_the_installed_runtime_after_stopping_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let worker = FakeWorker::start(OnShutdown::Close, 200, Some(marker.clone())).await;
    let runtime = script(
        dir.path(),
        "runtime",
        &format!(
            "touch '{}'\necho \"rendering $1\"\nexit 7",
            marker.display()
        ),
    );
    let bundle = bundle(dir.path());

    let ran = run_app(
        "ssr:start",
        &[
            ("INSSR_CHILD_SSR", &worker.url),
            ("INSSR_CHILD_BUNDLE", &path_str(&bundle)),
            ("INSSR_CHILD_RUNTIME", &path_str(&runtime)),
            ("INSSR_CHILD_ENSURE_RUNTIME", "1"),
        ],
    )
    .await;

    assert!(
        ran.out.contains(&format!("rendering {}", bundle.display())),
        "the installed runtime ran the installed bundle: {}",
        ran.out
    );
    assert_eq!(worker.paths(), vec!["/shutdown".to_owned()]);
    assert!(
        !worker.seen()[0].marker_existed,
        "stopped before it started"
    );
    assert_eq!(ran.code, 1, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err
            .contains(&format!("{RETURNED}the command exited with status 7")),
        "run_with_args reports the worker's exit status: {}",
        ran.err
    );
}

#[tokio::test]
async fn inssr_app_start_takes_the_runtime_flag_over_the_installed_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let configured = script(dir.path(), "configured", "echo configured");
    let flagged = script(dir.path(), "flagged", "echo flagged");
    let bundle = bundle(dir.path());

    let ran = run_app(
        &format!("ssr:start --runtime {}", path_str(&flagged)),
        &[
            ("INSSR_CHILD_SSR", &refusing_url()),
            ("INSSR_CHILD_BUNDLE", &path_str(&bundle)),
            ("INSSR_CHILD_RUNTIME", &path_str(&configured)),
        ],
    )
    .await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(ran.out.contains("flagged"), "{}", ran.out);
    assert!(!ran.out.contains("configured"), "{}", ran.out);
}

#[tokio::test]
async fn inssr_app_start_warns_and_uses_a_conventional_bundle_when_the_configured_one_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let conventional = dir.path().join("frontend/bootstrap/ssr");
    std::fs::create_dir_all(&conventional).unwrap();
    std::fs::write(conventional.join("ssr.mjs"), "// the SSR bundle\n").unwrap();
    let runtime = script(dir.path(), "runtime", "echo \"rendering $1\"");

    let ran = finish(
        app(
            "ssr:start",
            &[
                ("INSSR_CHILD_SSR", &refusing_url()),
                ("INSSR_CHILD_BUNDLE", "build/missing-ssr.js"),
                ("INSSR_CHILD_RUNTIME", &path_str(&runtime)),
            ],
        )
        .current_dir(dir.path()),
    )
    .await;

    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
    assert!(
        ran.err.contains(
            "Inertia SSR bundle not found at the configured path: \"build/missing-ssr.js\""
        ) && ran
            .err
            .contains("Using a default bundle instead: \"frontend/bootstrap/ssr/ssr.mjs\""),
        "both warnings name their path: {}",
        ran.err
    );
    assert!(
        ran.out.contains("rendering frontend/bootstrap/ssr/ssr.mjs"),
        "the conventional bundle ran: {}",
        ran.out
    );
}

/// Start `ssr:start` in a child, wait until the worker says it is ready,
/// send the child `signal`, and return what the child printed.
async fn signal_a_running_start(signal: nix::sys::signal::Signal) -> Ran {
    let dir = tempfile::tempdir().unwrap();
    // `wait` returns when a trapped signal arrives; the sleeper is killed so
    // nothing keeps the worker's output pipes open after it exits.
    let runtime = script(
        dir.path(),
        "runtime",
        "trap 'echo \"worker got TERM\"; kill $sleeper; exit 0' TERM\n\
         trap 'echo \"worker got INT\"; kill $sleeper; exit 0' INT\n\
         sleep 1000 >/dev/null 2>&1 &\n\
         sleeper=$!\n\
         echo ready\n\
         wait $sleeper",
    );
    let bundle = bundle(dir.path());
    let url = refusing_url();
    let mut child = app(
        "ssr:start",
        &[
            ("INSSR_CHILD_SSR", &url),
            ("INSSR_CHILD_BUNDLE", &path_str(&bundle)),
            ("INSSR_CHILD_RUNTIME", &path_str(&runtime)),
        ],
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .expect("spawn the child");

    let mut stderr = child.stderr.take().expect("the child's stderr");
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let mut lines = BufReader::new(child.stdout.take().expect("the child's stdout")).lines();
    let mut out = String::new();
    while let Some(line) = lines.next_line().await.expect("read the child's stdout") {
        out.push_str(&line);
        out.push('\n');
        if line == "ready" {
            break;
        }
    }
    assert!(
        out.contains("ready\n"),
        "the worker never said it was ready: {out} {}",
        errors.await.unwrap()
    );

    let pid = nix::unistd::Pid::from_raw(child.id().expect("the child runs") as i32);
    nix::sys::signal::kill(pid, signal).expect("signal the child");

    while let Some(line) = lines.next_line().await.expect("read the child's stdout") {
        out.push_str(&line);
        out.push('\n');
    }
    let status = child.wait().await.expect("wait for the child");
    Ran {
        code: status
            .code()
            .expect("the child exited, not killed by the signal"),
        out,
        err: errors.await.unwrap(),
    }
}

#[tokio::test]
async fn inssr_app_start_forwards_sigterm_to_the_worker() {
    let ran = signal_a_running_start(nix::sys::signal::Signal::SIGTERM).await;

    assert!(
        ran.out.contains("worker got TERM"),
        "the worker was sent SIGTERM: {} {}",
        ran.out,
        ran.err
    );
    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
}

#[tokio::test]
async fn inssr_app_start_forwards_sigint_to_the_worker() {
    let ran = signal_a_running_start(nix::sys::signal::Signal::SIGINT).await;

    assert!(
        ran.out.contains("worker got INT"),
        "the worker was sent SIGINT, the signal this process received: {} {}",
        ran.out,
        ran.err
    );
    assert_eq!(ran.code, 0, "out: {} err: {}", ran.out, ran.err);
}
