//! AGENT-005 and AGENT-017 in Chromium: the framework's default approval
//! page reaches the client's redirect URI with the default security headers
//! on, and a form under the global policy cannot, which is why every
//! consent page leaves its form's answer to a policy of its own or a
//! navigation of its own.
//!
//! `consent.spec.ts` runs against `framework/examples/agents_consent_host.rs`
//! with `consent.playwright.config.ts`. The binary's root declares this
//! module with one line, `#[path = "agents_browser/consent.rs"] mod
//! consent;`. Every test here is ignored: the suite runs with
//! `--run-ignored only`.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Host, run, target, workspace};

/// The framework example the specs run against.
const EXAMPLE: &str = "agents_consent_host";

/// Build the host and start it on a free port, under the default security
/// headers; its base URL.
fn start(work: &Path) -> Result<(Host, String), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(cargo);
    build
        .args(["build", "-p", "suprnova", "--example", EXAMPLE])
        .current_dir(workspace());
    run(build, &work.join("build.log"), Duration::from_secs(1800))?;
    let binary = target().join("debug/examples").join(EXAMPLE);
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
        // The preset under test is the one an application gets by default.
        .env_remove("SECURITY_HEADERS")
        .env_remove("SECURITY_HEADERS_CSP_REPORT_ONLY")
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

/// A run directory holding the spec and its configuration beside the Live
/// browser package's installed `node_modules`.
fn prepare(work: &Path) -> Result<PathBuf, String> {
    let suite = work.join("suite");
    fs::create_dir_all(&suite).map_err(|error| error.to_string())?;
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser");
    for file in [
        "package.json",
        "consent.playwright.config.ts",
        "consent.spec.ts",
    ] {
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
        let suite = prepare(&work)?;
        let (_host, url) = start(&work)?;
        let mut playwright = Command::new("node");
        playwright
            .arg(suite.join("node_modules/@playwright/test/cli.js"))
            .args([
                "test",
                "--config",
                "consent.playwright.config.ts",
                "--grep",
                &format!(" {name}$"),
            ])
            .current_dir(&suite)
            .env("CONSENT_BASE_URL", &url)
            .env("CONSENT_BROWSER_RESULTS", work.join("results"));
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
fn agent_017_default_approval_page_reaches_the_client_under_the_default_security_headers() {
    spec("agent_017_default_approval_page_reaches_the_client_under_the_default_security_headers");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_017_a_form_under_the_global_policy_cannot_reach_the_client() {
    spec("agent_017_a_form_under_the_global_policy_cannot_reach_the_client");
}
