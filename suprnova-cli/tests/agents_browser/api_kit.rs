//! AGENT-017 in Chromium for the API kit: the framework's default approval
//! and claim pages send a guest to the kit's `/login`, the guest signs in
//! there, and the decision reaches the client, under the security headers
//! the kit installs by default.
//!
//! `api-kit.spec.ts` runs through the suite's `playwright.config.ts`
//! against a fresh `--api` scaffold built against the in-tree framework.
//! The binary's root declares this module with one line,
//! `#[path = "agents_browser/api_kit.rs"] mod api_kit;`. The test is
//! ignored: the suite runs with `--run-ignored only`, with `TMPDIR` set to
//! a short directory, since Chromium refuses a long socket path.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Host, prepare_suite, run, target, workspace};

/// The account both flows sign in as. Its address is verified, which a
/// claim needs.
const EMAIL: &str = "ada@example.test";

/// The account's password.
const PASSWORD: &str = "api-kit-password-42";

/// The cases `api-kit.spec.ts` holds, each run on its own.
const CASES: [&str; 2] = [
    "agent_017_api_kit_approval_reaches_the_client_through_the_kit_sign_in",
    "agent_017_api_kit_claim_reaches_the_agent_through_the_kit_sign_in",
];

/// Scaffold the API kit named `name` in `work`, point it at the in-tree
/// framework with the workspace's lockfile, and build its binary into the
/// shared target directory; the project.
fn build(work: &Path, name: &str) -> Result<PathBuf, String> {
    let mut new = Command::new(env!("CARGO_BIN_EXE_suprnova"));
    new.args(["new", name, "--no-interaction", "--no-git", "--api"])
        .current_dir(work);
    run(new, &work.join("new.log"), Duration::from_secs(120))?;
    let project = work.join(name);

    // The in-tree framework, with every default feature the scaffold's
    // two `suprnova` lines name between them, and no workspace above.
    let manifest = project.join("Cargo.toml");
    let original = std::fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
    let framework = workspace().join("framework");
    let mut patched = String::new();
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            patched.push_str(&format!("suprnova = {{ path = {:?} }}\n", framework));
        } else {
            patched.push_str(line);
            patched.push('\n');
        }
    }
    patched.push_str("\n[workspace]\n");
    std::fs::write(&manifest, patched).map_err(|error| error.to_string())?;
    std::fs::copy(workspace().join("Cargo.lock"), project.join("Cargo.lock"))
        .map_err(|error| error.to_string())?;

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(cargo);
    build
        .args(["build", "--bin", name])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", target());
    run(
        build,
        &work.join("cargo-build.log"),
        Duration::from_secs(3 * 60 * 60),
    )?;
    Ok(project)
}

/// Start the application in `project` on a free port, under the security
/// headers it installs by default; the host and its base URL.
fn start(work: &Path, project: &Path, name: &str) -> Result<(Host, String), String> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let url = format!("http://127.0.0.1:{port}");
    let log = std::fs::File::create(work.join("server.log")).map_err(|error| error.to_string())?;
    let child = Command::new(target().join("debug").join(name))
        .arg("web:run")
        // The scaffold's `.env`, with its signing key, is read from here.
        .current_dir(project)
        .env("APP_ENV", "local")
        .env("APP_URL", &url)
        .env("SERVER_HOST", "127.0.0.1")
        .env("SERVER_PORT", port.to_string())
        .env("SESSION_SECURE", "false")
        .env("SESSION_COOKIE_PREFIX", "")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", work.join("app.sqlite").display()),
        )
        .env("MAIL_DRIVER", "memory")
        .env_remove("SECURITY_HEADERS")
        .env_remove("SECURITY_HEADERS_CSP_REPORT_ONLY")
        .stdout(log.try_clone().map_err(|error| error.to_string())?)
        .stderr(log)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("start the application: {error}"))?;
    Ok((Host(child), url))
}

/// Wait for the application, then give it the account both flows use:
/// registered through its JSON API, its address marked verified in
/// `app_users`, where agent auth reads it. The kit itself has no route that
/// verifies an address, so this stands in for the verification an
/// application adds.
fn prepare_account(host: &mut Host, url: &str, work: &Path) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| error.to_string())?;
        let started = Instant::now();
        loop {
            if let Some(status) = host.0.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "the application exited before it was ready: {status}"
                ));
            }
            // Any answer will do: the cases check what the answers are.
            let ready = client.get(format!("{url}/login")).send().await.is_ok();
            if ready {
                break;
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err("the application did not answer within 120 seconds".to_string());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let registered = client
            .post(format!("{url}/api/auth/register"))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(format!(r#"{{"email":"{EMAIL}","password":"{PASSWORD}"}}"#))
            .send()
            .await
            .map_err(|error| error.to_string())?;
        if registered.status().as_u16() != 202 {
            return Err(format!(
                "registration answered {}: {}",
                registered.status(),
                registered.text().await.unwrap_or_default()
            ));
        }
        let database = sea_orm::Database::connect(format!(
            "sqlite://{}?mode=rw",
            work.join("app.sqlite").display()
        ))
        .await
        .map_err(|error| error.to_string())?;
        let verified = sea_orm::ConnectionTrait::execute_unprepared(
            &database,
            &format!(
                "UPDATE app_users SET email_verified_at = '2026-01-01 00:00:00' \
                 WHERE email = '{EMAIL}'"
            ),
        )
        .await
        .map_err(|error| error.to_string())?;
        if verified.rows_affected() != 1 {
            return Err(format!("{EMAIL} has no account to verify"));
        }
        database.close().await.map_err(|error| error.to_string())
    })
}

/// AGENT-017: in Chromium, with the API kit's default security headers, a
/// guest the framework's approval page sends to the kit's `/login` signs
/// in, approves, and the decision reaches the client's redirect URI; and a
/// guest the claim page sends there signs in, claims the agent, and the
/// agent gets its token.
#[test]
#[ignore = "AGENT-017: needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_017_api_kit_consent_pages_complete_in_a_browser() {
    // One run directory, emptied first, so every run builds its scaffold
    // in the same place. With a new directory per run, and a failed run's
    // directory kept for inspection, a later run was served a binary built
    // from the earlier run's scaffold.
    let work = target().join("agents_browser").join("api_kit");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("create the run directory");
    let name = "agent_browser_api";
    let outcome = (|| -> Result<(), String> {
        let suite = prepare_suite(&work)?;
        let project = build(&work, name)?;
        let (mut host, url) = start(&work, &project, name)?;
        prepare_account(&mut host, &url, &work)?;
        for case in CASES {
            let log = work.join(format!("{case}.log"));
            let mut playwright = Command::new("node");
            playwright
                .arg(suite.join("node_modules/@playwright/test/cli.js"))
                .args(["test", "api-kit.spec.ts", "--grep", &format!(" {case}$")])
                .current_dir(&suite)
                .env("AGENTS_BASE_URL", &url)
                .env("AGENTS_BROWSER_RESULTS", work.join("results"))
                .env("API_KIT_EMAIL", EMAIL)
                .env("API_KIT_PASSWORD", PASSWORD);
            run(playwright, &log, Duration::from_secs(600))?;
            // A case the filter missed, or one the spec skipped, ends the
            // run without a failure; only a pass counts.
            let output = std::fs::read_to_string(&log).unwrap_or_default();
            if !output.contains("1 passed") || output.contains("skipped") {
                return Err(format!("{case} did not run and pass:\n{output}"));
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        panic!(
            "{error}\nserver log:\n{}\nrun directory: {}",
            std::fs::read_to_string(work.join("server.log")).unwrap_or_default(),
            work.display()
        );
    }
    let _ = std::fs::remove_dir_all(&work);
}
