//! AGENT-017 for the Live kit in Chromium: a scaffolded Live kit, served
//! under its default security headers, takes an MCP client's approval to
//! the client's redirect URI, claims an agent, refuses a wrong code and
//! revokes from its connected-agents page.
//!
//! The decision is the case a browser has to prove. The kit's policy
//! carries `form-action 'self'`, under which Chromium blocks a redirect
//! that answers a form post on another origin, so the Live kit answers the
//! decision with a page that navigates there by a meta refresh.
//!
//! The binary's root declares this module with one line,
//! `#[path = "agents_browser/live_kit.rs"] mod live_kit;`. Its spec is
//! `live-kit.spec.ts`, run with the suite's `playwright.config.ts` and the
//! Live browser package's installed Playwright. The test is ignored: the
//! suite runs with `--run-ignored only`, with `TMPDIR` set to a short
//! directory, since Chromium refuses a long socket path.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Host, prepare_suite, run, target, workspace};

/// The scaffold's name and so its binary's, which no other test builds
/// into the shared target directory.
const NAME: &str = "agent_browser_live";

/// The cases `live-kit.spec.ts` holds, each of which must run and pass.
const CASES: [&str; 4] = [
    "agent_017_live_kit_approval_reaches_the_client_under_the_default_security_headers",
    "agent_017_live_kit_claim_round_trip",
    "agent_017_live_kit_wrong_claim_code_shows_the_attempts_left",
    "agent_017_live_kit_connected_page_revokes",
];

/// Scaffold the Live kit into `work`, pointed at the in-tree framework
/// and this workspace's lock, so it reuses the framework the workspace
/// built.
fn scaffold(work: &Path) -> Result<PathBuf, String> {
    let output = Command::new(env!("CARGO_BIN_EXE_suprnova"))
        .args([
            "new",
            NAME,
            "--no-interaction",
            "--no-git",
            "--frontend",
            "live",
        ])
        .current_dir(work)
        .output()
        .map_err(|error| format!("run `suprnova new`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`suprnova new --frontend live` failed:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let project = work.join(NAME);
    let manifest = project.join("Cargo.toml");
    let original = fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
    let framework = workspace().join("framework");
    let mut rewritten = String::new();
    let mut replaced = false;
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!("suprnova = {{ path = {framework:?} }}\n"));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    if !replaced {
        return Err("the scaffold has no suprnova dependency".to_owned());
    }
    // The project lives under this workspace's target directory: without a
    // workspace table of its own, cargo would claim it as a member.
    rewritten.push_str("\n[workspace]\n");
    fs::write(&manifest, rewritten).map_err(|error| error.to_string())?;
    fs::copy(workspace().join("Cargo.lock"), project.join("Cargo.lock"))
        .map_err(|error| format!("copy Cargo.lock: {error}"))?;
    Ok(project)
}

/// The status of a `GET` of `path` on the local server, or `None` while it
/// does not answer.
fn status(port: u16, path: &str) -> Option<u16> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.split_whitespace().nth(1)?.parse().ok()
}

/// Start the built kit on a free port under its default security headers,
/// with mail written to the server's log as in development; the server and
/// its base URL.
fn start(project: &Path, log: &Path) -> Result<(Host, String), String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let database = project
        .parent()
        .ok_or("the run directory")?
        .join("app.sqlite");
    let file = fs::File::create(log).map_err(|error| error.to_string())?;
    let child = Command::new(target().join("debug").join(NAME))
        .arg("web:run")
        .current_dir(project)
        .env("APP_ENV", "local")
        .env("APP_URL", &url)
        .env("SERVER_HOST", "127.0.0.1")
        .env("SERVER_PORT", port.to_string())
        .env("SESSION_SECURE", "false")
        .env("SESSION_COOKIE_PREFIX", "")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", database.display()),
        )
        // The preset the kit ships in `.env`, named here so nothing else in
        // the environment changes it.
        .env("SECURITY_HEADERS", "balanced")
        .env("SECURITY_HEADERS_CSP_REPORT_ONLY", "false")
        // The `log` driver writes each mail to the log, where the spec
        // reads a verification link as a developer would.
        .env("MAIL_DRIVER", "log")
        .env("MAIL_FROM", "ops@example.test")
        .env("LOG_LEVEL", "info")
        .stdout(file.try_clone().map_err(|error| error.to_string())?)
        .stderr(file)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("start the kit: {error}"))?;
    let mut host = Host(child);
    let started = Instant::now();
    loop {
        if let Some(exit) = host.0.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("the kit exited before it was ready: {exit}"));
        }
        if status(port, "/login") == Some(200) {
            return Ok((host, url));
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err("the kit did not answer /login within 120 seconds".to_owned());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The titles of the report's specs whose every result passed.
fn passed(suites: &serde_json::Value, titles: &mut Vec<String>) {
    for suite in suites.as_array().into_iter().flatten() {
        for spec in suite["specs"].as_array().into_iter().flatten() {
            let results: Vec<&serde_json::Value> = spec["tests"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|test| test["results"].as_array().into_iter().flatten())
                .collect();
            if !results.is_empty()
                && results.iter().all(|result| result["status"] == "passed")
                && let Some(title) = spec["title"].as_str()
            {
                titles.push(title.to_owned());
            }
        }
        passed(&suite["suites"], titles);
    }
}

/// AGENT-017: the Live kit's consent pages in Chromium, under the kit's
/// default security headers. One scaffold and one server serve every case,
/// so the kit builds once.
#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_017_live_kit_consent_pages_in_chromium() {
    let work = target().join("agents_browser").join("live_kit");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).expect("create the run directory");
    let server_log = work.join("server.log");
    let report = work.join("report.json");
    let result = (|| -> Result<(), String> {
        let suite = prepare_suite(&work)?;
        let project = scaffold(&work)?;
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let mut build = Command::new(cargo);
        build
            .args(["build", "--bin", NAME])
            .current_dir(&project)
            .env("CARGO_TARGET_DIR", target());
        run(build, &work.join("build.log"), Duration::from_secs(3600))?;
        let (_server, url) = start(&project, &server_log)?;
        let mut playwright = Command::new("node");
        playwright
            .arg(suite.join("node_modules/@playwright/test/cli.js"))
            .args([
                "test",
                "--reporter=list,json",
                "--grep",
                " agent_017_live_kit_",
            ])
            .current_dir(&suite)
            .env("AGENTS_BASE_URL", &url)
            .env("AGENTS_BROWSER_RESULTS", work.join("results"))
            .env("PLAYWRIGHT_JSON_OUTPUT_NAME", &report)
            .env("LIVE_KIT_SERVER_LOG", &server_log);
        run(
            playwright,
            &work.join("playwright.log"),
            Duration::from_secs(900),
        )
    })();
    if let Err(error) = result {
        let log = fs::read_to_string(&server_log).unwrap_or_default();
        panic!("{error}\nserver log:\n{log}");
    }
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).expect("the Playwright report"))
            .expect("the report is JSON");
    let mut titles = Vec::new();
    passed(&report["suites"], &mut titles);
    let missing: Vec<&str> = CASES
        .iter()
        .copied()
        .filter(|case| !titles.iter().any(|title| title == case))
        .collect();
    assert!(
        missing.is_empty(),
        "these cases did not run and pass: {missing:#?}\nrun directory: {}",
        work.display()
    );
}
