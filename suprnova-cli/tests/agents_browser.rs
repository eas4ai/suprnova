//! `agents-browser`: Live tool actions registered with WebMCP in Chromium
//! (AGENT-012).
//!
//! Each test starts `framework/examples/agents_live_host.rs` on a free port
//! and runs one spec of `tests/agents_browser/agents.spec.ts` against it in
//! Playwright's Chromium with WebMCP enabled. The suite reuses the Live
//! browser package's installed Playwright (`@playwright/test` 1.62.1) and
//! downloads nothing.
//!
//! Run: `cargo nextest run -p suprnova-cli --test agents_browser --run-ignored only`,
//! with `TMPDIR` set to a short directory: Chromium's socket path is
//! refused under a long one.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The workspace root.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI crate sits in the workspace")
        .to_path_buf()
}

/// The target directory cargo builds into, as `CARGO_TARGET_TMPDIR`'s
/// parent names it.
fn target() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR sits inside the target directory")
        .to_path_buf()
}

/// Run `command` to completion within `timeout`, its output in `log`.
fn run(mut command: Command, log: &Path, timeout: Duration) -> Result<(), String> {
    let description = format!("{command:?}");
    let file = fs::File::create(log).map_err(|error| error.to_string())?;
    command
        .stdout(file.try_clone().map_err(|error| error.to_string())?)
        .stderr(file);
    let mut child = command
        .spawn()
        .map_err(|error| format!("{description}: {error}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let output = fs::read_to_string(log).unwrap_or_default();
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{description}: {status}\n{output}"))
            };
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let output = fs::read_to_string(log).unwrap_or_default();
            return Err(format!("{description}: timed out\n{output}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The host process, stopped when dropped.
struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Build the example host and start it on a free port; its base URL.
fn start_host(work: &Path) -> Result<(Host, String), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(cargo);
    build
        .args(["build", "-p", "suprnova", "--example", "agents_live_host"])
        .current_dir(workspace());
    run(build, &work.join("build.log"), Duration::from_secs(1800))?;
    let binary = target().join("debug/examples/agents_live_host");
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let log = fs::File::create(work.join("host.log")).map_err(|error| error.to_string())?;
    let child = Command::new(&binary)
        .current_dir(workspace().join("framework"))
        .env("SUPRNOVA_AGENTS_HOST_PORT", port.to_string())
        .env("APP_ENV", "local")
        .env("APP_URL", &url)
        .stdout(log.try_clone().map_err(|error| error.to_string())?)
        .stderr(log)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("{}: {error}", binary.display()))?;
    let mut host = Host(child);
    let started = Instant::now();
    loop {
        if let Some(status) = host.0.try_wait().map_err(|error| error.to_string())? {
            let output = fs::read_to_string(work.join("host.log")).unwrap_or_default();
            return Err(format!(
                "the host exited before it was ready: {status}\n{output}"
            ));
        }
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok((host, url));
        }
        if started.elapsed() > Duration::from_secs(60) {
            return Err("the host did not start listening within 60 seconds".to_string());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A run directory holding the suite's files beside the Live browser
/// package's installed `node_modules`, so Playwright resolves without a
/// download and the source tree gains no link.
fn prepare_suite(work: &Path) -> Result<PathBuf, String> {
    let suite = work.join("suite");
    fs::create_dir_all(&suite).map_err(|error| error.to_string())?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser");
    for file in ["package.json", "playwright.config.ts", "agents.spec.ts"] {
        fs::copy(source.join(file), suite.join(file))
            .map_err(|error| format!("copy {file}: {error}"))?;
    }
    let modules = workspace().join("crates/suprnova-live/browser/node_modules");
    if !modules.join("@playwright/test/cli.js").exists() {
        return Err(format!(
            "{} has no @playwright/test: install the Live browser package's dependencies first",
            modules.display()
        ));
    }
    let link = suite.join("node_modules");
    if !link.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&modules, &link).map_err(|error| error.to_string())?;
        #[cfg(not(unix))]
        return Err("the agents browser suite runs on Unix".to_string());
    }
    Ok(suite)
}

/// Run the spec named `name` against a fresh host.
fn spec(name: &str) {
    let work = target().join("agents_browser").join(name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).expect("create the run directory");
    let result = (|| -> Result<(), String> {
        let suite = prepare_suite(&work)?;
        let (_host, url) = start_host(&work)?;
        let mut playwright = Command::new("node");
        playwright
            .arg(suite.join("node_modules/@playwright/test/cli.js"))
            .args(["test", "--grep", &format!(" {name}$")])
            .current_dir(&suite)
            .env("AGENTS_BASE_URL", &url)
            .env("AGENTS_BROWSER_RESULTS", work.join("results"));
        run(
            playwright,
            &work.join("playwright.log"),
            Duration::from_secs(600),
        )
    })();
    if let Err(error) = result {
        let host = fs::read_to_string(work.join("host.log")).unwrap_or_default();
        panic!("{name} failed:\n{error}\nhost log:\n{host}");
    }
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_mounted_tool_actions_register_with_model_context() {
    spec("agent_012_mounted_tool_actions_register_with_model_context");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tool_names_carry_the_live_key() {
    spec("agent_012_tool_names_carry_the_live_key");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tools_leave_with_their_component() {
    spec("agent_012_tools_leave_with_their_component");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tools_leave_on_navigation() {
    spec("agent_012_tools_leave_on_navigation");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tool_call_sends_the_island_snapshot_and_passes_the_csrf_and_session_checks() {
    spec("agent_012_tool_call_sends_the_island_snapshot_and_passes_the_csrf_and_session_checks");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tool_call_is_authorized() {
    spec("agent_012_tool_call_is_authorized");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tool_call_runs_the_execution_phases_of_a_click() {
    spec("agent_012_tool_call_runs_the_execution_phases_of_a_click");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_012_tool_result_carries_only_agent_visible_fields() {
    spec("agent_012_tool_result_carries_only_agent_visible_fields");
}
