//! The `suprnova` CLI's `ssr:start`, `ssr:stop` and `ssr:check`.
//!
//! The `inssr_` tests (PAR-061) run the CLI binary against a fake worker, a
//! TCP listener the test drives, and a fake runtime, a shell script written
//! to a temporary directory. Where the CLI's output is compared with what
//! `suprnova::console::ssr` prints in this process for the same
//! configuration, the test is the proof that the CLI runs the shared
//! implementation the application binary runs.
//!
//! The end-to-end proof for T31 - `suprnova new` -> `vite build --ssr` ->
//! `suprnova ssr:start` -> a hard-navigation HTML response contains the
//! SSR-rendered body - is `#[ignore]`d: it needs Node/npm and network access
//! for `npm install`, neither of which the normal `cargo test --workspace`
//! run can assume. Run it explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test ssr_e2e -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_suprnova"))
}

fn workspace_framework_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("suprnova-cli must have a workspace parent")
        .join("framework")
}

fn run_ok(mut cmd: Command, what: &str) {
    let status = cmd.status().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(status.success(), "{what} exited with {status}");
}

/// Point the scaffold at the in-tree framework crate - the published
/// `suprnova` tag doesn't carry this task's changes yet. Mirrors
/// `scaffold_snapshot.rs::patch_local_suprnova`.
fn patch_local_suprnova(project: &Path) {
    let cargo_toml = project.join("Cargo.toml");
    let original = std::fs::read_to_string(&cargo_toml).expect("read scaffolded Cargo.toml");
    let mut rewritten = String::with_capacity(original.len());
    let mut replaced = false;
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                workspace_framework_dir().display()
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "scaffolded Cargo.toml must declare a suprnova dependency"
    );
    std::fs::write(&cargo_toml, rewritten).expect("write patched Cargo.toml");
}

/// SSR is off by default (Design Note 4) - this proof has to opt in
/// itself, the same way any app adopting SSR would.
fn enable_ssr(project: &Path) {
    let path = project.join("src/bootstrap.rs");
    let original = std::fs::read_to_string(&path).expect("read scaffolded bootstrap.rs");
    let needle = "InertiaConfig::new().frontend(Frontend::Svelte))";
    assert!(
        original.contains(needle),
        "scaffolded bootstrap.rs no longer matches the expected Inertia::install call \
         (template drifted). bootstrap.rs:\n{original}"
    );
    let patched = original.replace(
        needle,
        "InertiaConfig::new().frontend(Frontend::Svelte).ssr(\"http://127.0.0.1:13714\"))",
    );
    std::fs::write(&path, patched).expect("write patched bootstrap.rs");
}

/// Poll `ssr:check` until it reports the worker healthy or `budget` runs out.
fn wait_for_ssr_healthy(budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let ok = Command::new(cli_binary())
            .args(["ssr:check", "--timeout-ms", "500"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "SSR worker never became healthy within {budget:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn get_body(addr: std::net::SocketAddr) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .expect("write request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let text = String::from_utf8_lossy(&response).into_owned();
    text.split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string()
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "e2e: needs npm/node, runs `vite build --ssr`, boots two processes - run manually"]
fn scaffolded_ssr_entry_produces_server_rendered_html() {
    let tmp = TempDir::new().unwrap();
    let project_name = "ssr_e2e_app";

    let mut new_cmd = Command::new(cli_binary());
    new_cmd
        .args([
            "new",
            project_name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path());
    run_ok(new_cmd, "suprnova new");

    let project = tmp.path().join(project_name);
    let frontend = project.join("frontend");
    patch_local_suprnova(&project);
    enable_ssr(&project);

    let mut npm_install = Command::new("npm");
    npm_install.arg("install").current_dir(&frontend);
    run_ok(npm_install, "npm install");

    let mut npm_build = Command::new("npm");
    npm_build.args(["run", "build"]).current_dir(&frontend);
    run_ok(npm_build, "npm run build");

    let mut npm_build_ssr = Command::new("npm");
    npm_build_ssr
        .args(["run", "build:ssr"])
        .current_dir(&frontend);
    run_ok(npm_build_ssr, "npm run build:ssr");

    assert!(
        frontend.join("bootstrap/ssr/ssr.js").exists(),
        "vite build --ssr must produce frontend/bootstrap/ssr/ssr.js"
    );

    let _ssr_worker = KillOnDrop(
        Command::new(cli_binary())
            .arg("ssr:start")
            .current_dir(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ssr:start"),
    );
    wait_for_ssr_healthy(Duration::from_secs(30));

    let backend_port: u16 = 18765;
    let addr: std::net::SocketAddr = format!("127.0.0.1:{backend_port}").parse().unwrap();
    let mut backend_cmd = Command::new(env!("CARGO"));
    backend_cmd
        .args(["run", "--bin", project_name, "--", "serve"])
        .current_dir(&project)
        .env("APP_ENV", "production")
        .env("SERVER_PORT", backend_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _backend = KillOnDrop(backend_cmd.spawn().expect("spawn backend"));

    // The backend is a debug `cargo run` compile + boot - give it real
    // time rather than guessing an exact readiness signal.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "backend never started listening on {addr}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    let body = get_body(addr);
    assert!(
        body.contains("data-server-rendered=\"true\""),
        "hard navigation must be server-rendered; body:\n{body}"
    );
}

// ---------------------------------------------------------------------------
// PAR-061: the CLI runs the shared implementation
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod inssr {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use suprnova::SsrConfig;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    use super::cli_binary;

    /// The environment the CLI's SSR commands read, cleared for every run
    /// so the developer's own shell cannot change what a test sees.
    const SSR_ENV: [&str; 4] = [
        "SUPRNOVA_SSR_URL",
        "SUPRNOVA_SSR_RUNTIME",
        "SUPRNOVA_SSR_BUNDLE",
        "SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS",
    ];

    /// What the fake worker does with a request.
    #[derive(Clone, Copy)]
    enum Reply {
        /// Close the connection without an answer, as the Inertia SSR
        /// server does when `/shutdown` makes it exit.
        Close,
        /// Answer with this status.
        Status(u16),
        /// Hold the connection open and never answer.
        Hang,
    }

    /// One request the fake worker received.
    #[derive(Debug, Clone)]
    struct Seen {
        path: String,
        /// Whether the marker file existed when the request arrived: the
        /// fake runtime creates it, so `false` means no worker had started.
        marker_existed: bool,
    }

    struct FakeWorker {
        url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl FakeWorker {
        async fn start(shutdown: Reply, health: Reply, marker: Option<PathBuf>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the fake worker");
            let url = format!("http://{}", listener.local_addr().expect("local address"));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let record = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut held = Vec::new();
                while let Ok((mut stream, _)) = listener.accept().await {
                    let path = read_request_path(&mut stream).await;
                    record.lock().unwrap().push(Seen {
                        path: path.clone(),
                        marker_existed: marker.as_ref().is_some_and(|m| m.exists()),
                    });
                    let reply = match path.as_str() {
                        "/shutdown" => shutdown,
                        "/health" => health,
                        _ => Reply::Status(404),
                    };
                    match reply {
                        Reply::Close => drop(stream),
                        Reply::Hang => held.push(stream),
                        Reply::Status(status) => {
                            let response = format!(
                                "HTTP/1.1 {status} Fake\r\ncontent-length: 0\r\n\
                                 connection: close\r\n\r\n"
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.shutdown().await;
                        }
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

    /// Read a request's head and return its path. The whole head is read,
    /// so closing the connection afterwards is a clean close, not a reset.
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

    /// A loopback URL nothing listens on, so a connection is refused at
    /// once. A privileged port, because no test can bind one.
    fn refusing_url() -> String {
        for port in 1..1024u16 {
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
                return format!("http://{addr}");
            }
        }
        panic!("every privileged loopback port accepted a connection");
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the script executable");
        path
    }

    fn bundle(dir: &Path) -> PathBuf {
        let path = dir.join("ssr.js");
        std::fs::write(&path, "// the SSR bundle\n").expect("write the bundle");
        path
    }

    fn path_str(path: &Path) -> String {
        path.to_str().expect("a UTF-8 temporary path").to_owned()
    }

    /// The CLI with `args`, in `cwd`, with only the SSR environment given.
    fn cli(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(cli_binary());
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .kill_on_drop(true);
        for name in SSR_ENV {
            command.env_remove(name);
        }
        for (name, value) in env {
            command.env(name, value);
        }
        command
    }

    /// What a command exited with and printed.
    #[derive(Debug, PartialEq, Eq)]
    struct Ran {
        code: i32,
        out: String,
        err: String,
    }

    async fn run(command: &mut tokio::process::Command) -> Ran {
        let output = command.output().await.expect("run the CLI");
        Ran {
            code: output
                .status
                .code()
                .expect("the CLI exited, not killed by a signal"),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn ran(code: i32, out: Vec<u8>, err: Vec<u8>) -> Ran {
        Ran {
            code,
            out: String::from_utf8(out).expect("UTF-8 output"),
            err: String::from_utf8(err).expect("UTF-8 errors"),
        }
    }

    /// What the shared `stop` prints in this process for `config`.
    async fn shared_stop(config: &SsrConfig, graceful: bool) -> Ran {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = suprnova::console::ssr::stop(config, graceful, &mut out, &mut err)
            .await
            .expect("ssr::stop");
        ran(code, out, err)
    }

    /// What the shared `check` prints in this process for `config`.
    async fn shared_check(config: &SsrConfig) -> Ran {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = suprnova::console::ssr::check(config, &mut out, &mut err)
            .await
            .expect("ssr::check");
        ran(code, out, err)
    }

    /// The configuration the CLI builds from `--url` and `--timeout-ms`.
    fn flags_config(url: &str, timeout_ms: u64) -> SsrConfig {
        SsrConfig {
            enabled: true,
            url: url.to_owned(),
            timeout: Duration::from_millis(timeout_ms),
            ..SsrConfig::default()
        }
    }

    // -- ssr:start ----------------------------------------------------------

    #[tokio::test]
    async fn inssr_cli_start_fails_without_a_bundle() {
        let dir = tempfile::tempdir().unwrap();

        let ran = run(&mut cli(
            &["ssr:start"],
            dir.path(),
            &[("SUPRNOVA_SSR_URL", &refusing_url())],
        ))
        .await;

        assert_eq!(ran.code, 1, "{ran:?}");
        assert!(
            ran.err.contains("Inertia SSR bundle not found.")
                && ran.err.contains("SUPRNOVA_SSR_BUNDLE"),
            "the shared message, naming how to set the bundle: {}",
            ran.err
        );
    }

    #[tokio::test]
    async fn inssr_cli_start_fails_naming_a_configured_bundle_that_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-ssr.js");

        let ran = run(&mut cli(
            &["ssr:start", "--bundle", &path_str(&missing)],
            dir.path(),
            &[("SUPRNOVA_SSR_URL", &refusing_url())],
        ))
        .await;

        assert_eq!(ran.code, 1, "{ran:?}");
        assert_eq!(
            ran.err,
            format!(
                "Inertia SSR bundle not found at the configured path: \"{}\"\n",
                missing.display()
            )
        );
    }

    #[tokio::test]
    async fn inssr_cli_start_warns_and_runs_a_conventional_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let conventional = dir.path().join("frontend/bootstrap/ssr");
        std::fs::create_dir_all(&conventional).unwrap();
        std::fs::write(conventional.join("ssr.mjs"), "// the SSR bundle\n").unwrap();
        let runtime = script(dir.path(), "runtime", "echo \"rendering $1\"");

        let ran = run(&mut cli(
            &["ssr:start"],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &refusing_url()),
                ("SUPRNOVA_SSR_BUNDLE", "build/missing-ssr.js"),
                ("SUPRNOVA_SSR_RUNTIME", &path_str(&runtime)),
            ],
        ))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(
            ran.err,
            "Inertia SSR bundle not found at the configured path: \"build/missing-ssr.js\"\n\
             Using a default bundle instead: \"frontend/bootstrap/ssr/ssr.mjs\"\n"
        );
        assert_eq!(ran.out, "rendering frontend/bootstrap/ssr/ssr.mjs\n");
    }

    #[tokio::test]
    async fn inssr_cli_start_refuses_a_missing_runtime_when_ensure_runtime_exists() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = bundle(dir.path());

        let ran = run(&mut cli(
            &[
                "ssr:start",
                "--bundle",
                &path_str(&bundle),
                "--runtime",
                "suprnova-inssr-no-such-runtime",
            ],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &refusing_url()),
                ("SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS", "true"),
            ],
        ))
        .await;

        assert_eq!(ran.code, 1, "{ran:?}");
        assert_eq!(
            ran.err,
            "SSR runtime \"suprnova-inssr-no-such-runtime\" could not be found.\n"
        );
    }

    #[tokio::test]
    async fn inssr_cli_start_finds_a_runtime_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        script(&bin, "inssr-fake-node", "echo \"ran $1\"");
        let bundle = bundle(dir.path());
        let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .unwrap();

        let ran = run(cli(
            &["ssr:start", "--bundle", &path_str(&bundle)],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &refusing_url()),
                ("SUPRNOVA_SSR_RUNTIME", "inssr-fake-node"),
                ("SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS", "1"),
            ],
        )
        .env("PATH", path))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(ran.out, format!("ran {}\n", bundle.display()));
    }

    #[tokio::test]
    async fn inssr_cli_start_rejects_an_ensure_runtime_exists_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();

        let ran = run(&mut cli(
            &["ssr:start"],
            dir.path(),
            &[("SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS", "sometimes")],
        ))
        .await;

        assert_eq!(ran.code, 1, "{ran:?}");
        assert!(
            ran.err.contains("SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS") && ran.err.contains("sometimes"),
            "{}",
            ran.err
        );
    }

    #[tokio::test]
    async fn inssr_cli_start_stops_a_running_worker_first() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let worker =
            FakeWorker::start(Reply::Close, Reply::Status(200), Some(marker.clone())).await;
        let runtime = script(
            dir.path(),
            "runtime",
            &format!("touch '{}'\necho started", marker.display()),
        );
        let bundle = bundle(dir.path());

        let ran = run(&mut cli(
            &["ssr:start", "--bundle", &path_str(&bundle)],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &worker.url),
                ("SUPRNOVA_SSR_RUNTIME", &path_str(&runtime)),
            ],
        ))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(worker.paths(), vec!["/shutdown".to_owned()]);
        assert!(
            !worker.seen()[0].marker_existed,
            "the shutdown came before the new worker started"
        );
        assert_eq!(ran.out, "started\n");
    }

    #[tokio::test]
    async fn inssr_cli_start_reports_stderr_and_exits_with_the_workers_code() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = bundle(dir.path());
        let runtime = script(
            dir.path(),
            "runtime",
            "echo rendering\necho 'a problem' >&2\nexit 3",
        );

        let ran = run(&mut cli(
            &["ssr:start", "--bundle", &path_str(&bundle)],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &refusing_url()),
                ("SUPRNOVA_SSR_RUNTIME", &path_str(&runtime)),
            ],
        ))
        .await;

        assert_eq!(ran.code, 3, "{ran:?}");
        assert_eq!(ran.out, "rendering\n");
        assert_eq!(ran.err, "a problem\n");
    }

    /// Start `ssr:start`, wait until the worker says it is ready, send the
    /// CLI `signal`, and return what it printed.
    ///
    /// The worker writes its process ids to a file. A CLI that did not
    /// forward the signal dies of it and leaves the worker running, holding
    /// the output pipe open; the worker is then killed by those ids so the
    /// test can finish and report.
    async fn signal_a_running_start(signal: nix::sys::signal::Signal) -> Ran {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;

        let dir = tempfile::tempdir().unwrap();
        let pids = dir.path().join("pids");
        let runtime = script(
            dir.path(),
            "runtime",
            &format!(
                "trap 'echo \"worker got TERM\"; kill $sleeper; exit 0' TERM\n\
                 trap 'echo \"worker got INT\"; kill $sleeper; exit 0' INT\n\
                 sleep 1000 >/dev/null 2>&1 &\n\
                 sleeper=$!\n\
                 echo \"$$ $sleeper\" > '{}'\n\
                 echo ready\n\
                 wait $sleeper",
                pids.display()
            ),
        );
        let bundle = bundle(dir.path());
        let mut child = cli(
            &["ssr:start", "--bundle", &path_str(&bundle)],
            dir.path(),
            &[
                ("SUPRNOVA_SSR_URL", &refusing_url()),
                ("SUPRNOVA_SSR_RUNTIME", &path_str(&runtime)),
            ],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the CLI");

        let mut stderr = child.stderr.take().expect("the CLI's stderr");
        let errors = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });
        let mut lines = BufReader::new(child.stdout.take().expect("the CLI's stdout")).lines();
        let mut out = String::new();
        while let Some(line) = lines.next_line().await.expect("read the CLI's stdout") {
            out.push_str(&line);
            out.push('\n');
            if line == "ready" {
                break;
            }
        }
        assert!(
            out.contains("ready\n"),
            "the worker never said it was ready: {out}"
        );

        let cli_pid = Pid::from_raw(child.id().expect("the CLI runs") as i32);
        kill(cli_pid, signal).expect("signal the CLI");
        let status = child.wait().await.expect("wait for the CLI");
        if status.code().is_none() {
            let ids = std::fs::read_to_string(&pids).unwrap_or_default();
            for id in ids
                .split_whitespace()
                .filter_map(|id| id.parse::<i32>().ok())
            {
                let _ = kill(Pid::from_raw(id), Signal::SIGKILL);
            }
        }
        while let Some(line) = lines.next_line().await.expect("read the CLI's stdout") {
            out.push_str(&line);
            out.push('\n');
        }
        let err = errors.await.unwrap();
        Ran {
            code: status
                .code()
                .unwrap_or_else(|| panic!("the CLI died of {signal}: {out} {err}")),
            out,
            err,
        }
    }

    #[tokio::test]
    async fn inssr_cli_start_forwards_sigterm_to_the_worker() {
        let ran = signal_a_running_start(nix::sys::signal::Signal::SIGTERM).await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(ran.out, "ready\nworker got TERM\n");
    }

    #[tokio::test]
    async fn inssr_cli_start_forwards_sigint_to_the_worker() {
        let ran = signal_a_running_start(nix::sys::signal::Signal::SIGINT).await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(ran.out, "ready\nworker got TERM\n");
    }

    // -- ssr:stop -----------------------------------------------------------

    #[tokio::test]
    async fn inssr_cli_stop_succeeds_when_the_worker_closes_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let worker = FakeWorker::start(Reply::Close, Reply::Status(200), None).await;

        let ran = run(&mut cli(
            &["ssr:stop", "--url", &worker.url],
            dir.path(),
            &[],
        ))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(ran.out, "Inertia SSR server stopped.\n");
        assert_eq!(worker.paths(), vec!["/shutdown".to_owned()]);
        assert_eq!(
            ran,
            shared_stop(&flags_config(&worker.url, 2000), false).await,
            "the CLI prints what the shared stop prints"
        );
    }

    #[tokio::test]
    async fn inssr_cli_stop_with_graceful_succeeds_when_no_worker_runs() {
        let dir = tempfile::tempdir().unwrap();
        let url = refusing_url();

        let graceful = run(&mut cli(
            &["ssr:stop", "--graceful"],
            dir.path(),
            &[("SUPRNOVA_SSR_URL", &url)],
        ))
        .await;
        assert_eq!(graceful.code, 0, "{graceful:?}");
        assert_eq!(graceful.out, "Inertia SSR server is not running.\n");
        assert_eq!(graceful, shared_stop(&flags_config(&url, 2000), true).await);

        let plain = run(&mut cli(&["ssr:stop", "--url", &url], dir.path(), &[])).await;
        assert_eq!(plain.code, 1, "{plain:?}");
        assert_eq!(plain.err, "Unable to connect to Inertia SSR server.\n");
        assert_eq!(plain, shared_stop(&flags_config(&url, 2000), false).await);
    }

    #[tokio::test]
    async fn inssr_cli_stop_fails_when_the_worker_keeps_running() {
        let dir = tempfile::tempdir().unwrap();
        for (shutdown, timeout) in [(Reply::Status(200), "2000"), (Reply::Hang, "200")] {
            let worker = FakeWorker::start(shutdown, Reply::Status(200), None).await;

            let ran = run(&mut cli(
                &[
                    "ssr:stop",
                    "--graceful",
                    "--url",
                    &worker.url,
                    "--timeout-ms",
                    timeout,
                ],
                dir.path(),
                &[],
            ))
            .await;

            assert_eq!(ran.code, 1, "{ran:?}");
            assert_eq!(ran.err, "Unable to connect to Inertia SSR server.\n");
        }
    }

    // -- ssr:check ----------------------------------------------------------

    #[tokio::test]
    async fn inssr_cli_check_prints_what_the_shared_check_prints() {
        let dir = tempfile::tempdir().unwrap();
        for health in [Reply::Status(500), Reply::Hang] {
            let worker = FakeWorker::start(Reply::Close, health, None).await;

            let ran = run(&mut cli(
                &["ssr:check", "--url", &worker.url, "--timeout-ms", "200"],
                dir.path(),
                &[],
            ))
            .await;

            assert_eq!(ran.code, 1, "a worker whose /health fails: {ran:?}");
            assert_eq!(
                ran,
                shared_check(&flags_config(&worker.url, 200)).await,
                "the CLI prints what the shared check prints"
            );
        }
    }

    /// Needs the HTTP gateway's health check (`HttpGateway::is_healthy`,
    /// PAR-060): without it the gateway has none, and `ssr:check` says so.
    #[tokio::test]
    async fn inssr_cli_check_succeeds_against_a_healthy_worker() {
        let dir = tempfile::tempdir().unwrap();
        let worker = FakeWorker::start(Reply::Close, Reply::Status(200), None).await;

        let ran = run(&mut cli(
            &["ssr:check"],
            dir.path(),
            &[("SUPRNOVA_SSR_URL", &worker.url)],
        ))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        assert_eq!(ran.out, "Inertia SSR server is running.\n");
        assert_eq!(worker.paths(), vec!["/health".to_owned()]);
    }

    #[tokio::test]
    async fn inssr_cli_check_and_stop_refuse_a_url_without_a_scheme() {
        let dir = tempfile::tempdir().unwrap();
        for command in ["ssr:check", "ssr:stop"] {
            let ran = run(&mut cli(
                &[command, "--url", "127.0.0.1:13714"],
                dir.path(),
                &[],
            ))
            .await;

            assert_eq!(ran.code, 1, "{command}: {ran:?}");
            assert!(
                ran.err.contains("http://") && ran.err.contains("\"127.0.0.1:13714\""),
                "{command}: the error names the URL and the scheme it needs: {}",
                ran.err
            );
        }
    }
}
