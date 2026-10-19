//! AGENT-016 in Chromium: a Rust front end compiled to WebAssembly with
//! `suprnova-webmcp` registers a typed page tool, removes it when its
//! handle is dropped, and calls the page's server tools with the user's
//! session and CSRF token.
//!
//! Each test builds `crates/suprnova-webmcp/examples/browser_fixture.rs`
//! for `wasm32-unknown-unknown`, runs `wasm-bindgen --target web` on it,
//! starts `framework/examples/agents_wasm_host.rs` serving that output,
//! and runs one spec of `tests/agents_browser/wasm.spec.ts` against it in
//! Playwright's Chromium with WebMCP enabled.
//!
//! The gate provisions nothing at run time, so the machine needs, once:
//! the `wasm32-unknown-unknown` target for the pinned toolchain, and
//! `wasm-bindgen-cli` at the version of `wasm-bindgen` in `Cargo.lock`. A
//! missing piece fails the test with a message naming it.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Host, run, target, workspace};

/// The `wasm-bindgen` the lock pins; the CLI must match it exactly.
fn locked_wasm_bindgen() -> Result<String, String> {
    let lock = fs::read_to_string(workspace().join("Cargo.lock"))
        .map_err(|error| format!("read Cargo.lock: {error}"))?;
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"wasm-bindgen\"" {
            return lines
                .next()
                .and_then(|version| version.strip_prefix("version = \""))
                .and_then(|version| version.strip_suffix('"'))
                .map(str::to_string)
                .ok_or_else(|| "Cargo.lock names wasm-bindgen without a version".to_string());
        }
    }
    Err("Cargo.lock holds no wasm-bindgen".to_string())
}

/// Build the fixture for the browser and bind it; the directory holding
/// `browser_fixture.js` and its `.wasm`.
fn build_fixture(work: &Path) -> Result<PathBuf, String> {
    let wanted = locked_wasm_bindgen()?;
    let version = Command::new("wasm-bindgen")
        .arg("--version")
        .output()
        .map_err(|error| {
            format!(
                "wasm-bindgen is not installed ({error}): install it once with \
                 `cargo install wasm-bindgen-cli --version {wanted} --locked`"
            )
        })?;
    let version = String::from_utf8_lossy(&version.stdout).trim().to_string();
    if version != format!("wasm-bindgen {wanted}") {
        return Err(format!(
            "`wasm-bindgen --version` says `{version}`, but the lock pins {wanted}: install it with \
             `cargo install wasm-bindgen-cli --version {wanted} --locked`"
        ));
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(cargo);
    build
        .args([
            "build",
            "-p",
            "suprnova-webmcp",
            "--example",
            "browser_fixture",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(workspace());
    run(
        build,
        &work.join("wasm-build.log"),
        Duration::from_secs(1800),
    )
    .map_err(|error| {
        format!(
            "{error}\nIf the target is missing, add it once with \
             `rustup target add wasm32-unknown-unknown`."
        )
    })?;
    let wasm = target().join("wasm32-unknown-unknown/debug/examples/browser_fixture.wasm");
    let pkg = work.join("pkg");
    let mut bind = Command::new("wasm-bindgen");
    bind.args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(&pkg)
        .arg(&wasm);
    run(
        bind,
        &work.join("wasm-bindgen.log"),
        Duration::from_secs(300),
    )?;
    Ok(pkg)
}

/// Build the host and start it on a free port, serving `pkg`; its base URL.
fn start_wasm_host(work: &Path, pkg: &Path) -> Result<(Host, String), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(cargo);
    build
        .args(["build", "-p", "suprnova", "--example", "agents_wasm_host"])
        .current_dir(workspace());
    run(build, &work.join("build.log"), Duration::from_secs(1800))?;
    let binary = target().join("debug/examples/agents_wasm_host");
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let log = fs::File::create(work.join("host.log")).map_err(|error| error.to_string())?;
    let child = Command::new(&binary)
        .current_dir(workspace().join("framework"))
        .env("SUPRNOVA_AGENTS_HOST_PORT", port.to_string())
        .env("SUPRNOVA_WASM_PKG_DIR", pkg)
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

/// A run directory holding the WebAssembly spec beside the Live browser
/// package's installed `node_modules`, so Playwright resolves without a
/// download.
fn prepare_wasm_suite(work: &Path) -> Result<PathBuf, String> {
    let suite = work.join("suite");
    fs::create_dir_all(&suite).map_err(|error| error.to_string())?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser");
    for file in ["package.json", "wasm.spec.ts"] {
        fs::copy(source.join(file), suite.join(file))
            .map_err(|error| format!("copy {file}: {error}"))?;
    }
    // The shared config names its specs; this run holds only the
    // WebAssembly one, so the copy matches it too.
    let config = fs::read_to_string(source.join("playwright.config.ts"))
        .map_err(|error| format!("read playwright.config.ts: {error}"))?
        .replace("testMatch: 'agents.spec.ts'", "testMatch: '*.spec.ts'");
    fs::write(suite.join("playwright.config.ts"), config)
        .map_err(|error| format!("write playwright.config.ts: {error}"))?;
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

/// Run the WebAssembly spec named `name` against a fresh host.
fn wasm_spec(name: &str) {
    let work = target().join("agents_browser").join(name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).expect("create the run directory");
    let result = (|| -> Result<(), String> {
        let suite = prepare_wasm_suite(&work)?;
        let pkg = build_fixture(&work)?;
        let (_host, url) = start_wasm_host(&work, &pkg)?;
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
#[ignore = "needs Playwright's Chromium 1.62.1 and wasm-bindgen-cli; run with --run-ignored only"]
fn agent_016_wasm_registers_a_typed_tool() {
    wasm_spec("agent_016_wasm_registers_a_typed_tool");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1 and wasm-bindgen-cli; run with --run-ignored only"]
fn agent_016_dropping_the_handle_unregisters_the_tool() {
    wasm_spec("agent_016_dropping_the_handle_unregisters_the_tool");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1 and wasm-bindgen-cli; run with --run-ignored only"]
fn agent_016_wasm_server_tool_call_carries_session_csrf_and_header() {
    wasm_spec("agent_016_wasm_server_tool_call_carries_session_csrf_and_header");
}
