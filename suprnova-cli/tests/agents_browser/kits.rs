//! AGENT-015, and AGENT-014 for the Inertia kits: the React, Vue and Svelte
//! kits' page tools, component tools and form tools.
//!
//! The binary's root declares this module with one line,
//! `#[path = "agents_browser/kits.rs"] mod kits;`: a test crate's root
//! resolves `mod kits;` beside itself, not under `agents_browser/`. The
//! modules below sit beside this file.
//!
//! Every test here is ignored: the `agents-browser` mechanism runs the
//! binary with `--run-ignored only`.

#[path = "schemas.rs"]
mod schemas;

#[path = "../kit_support/mod.rs"]
mod kit_support;

use std::path::Path;
use std::time::Duration;

use kit_support::{KITS, read, run, scaffold};
use tokio::process::Command as AsyncCommand;

/// The template a kit file must equal byte for byte.
fn template(relative: &str) -> String {
    read(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/templates/files/frontend")
            .join(relative),
    )
}

/// Every Inertia kit ships the agent helpers: the shared engine and the
/// page tools installer byte for byte, the kit's own hooks over them, the
/// form request schemas `generate-types` writes for the kit, and an entry
/// that installs the page tools before the application starts, so the
/// first page's `navigate` event registers its tools.
#[test]
#[ignore = "AGENT-015 evidence: the agents-browser mechanism runs --run-ignored only"]
fn agent_015_inertia_kits_ship_the_agent_helpers() {
    for kit in KITS {
        let parent = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
        let project =
            scaffold(parent.path(), "agent_kit", kit).unwrap_or_else(|error| panic!("{error}"));
        let file = |relative: &str| read(&project.join("frontend/src").join(relative));

        assert_eq!(
            file("lib/agents/core.ts"),
            template("agents/core.ts.tpl"),
            "{kit}: core.ts"
        );
        assert_eq!(
            file("lib/agents/inertia.ts"),
            template("agents/inertia.ts.tpl"),
            "{kit}: inertia.ts"
        );
        assert_eq!(
            file("types/tool-schemas.ts"),
            template("types/tool-schemas.ts.tpl"),
            "{kit}: tool-schemas.ts"
        );
        let hooks = file("lib/agents/index.ts");
        for needle in [
            "from './core'",
            "export { installPageTools } from './inertia'",
            "export function usePageTool",
            "export function useFormTool",
        ] {
            assert!(
                hooks.contains(needle),
                "{kit}: index.ts lacks {needle}:\n{hooks}"
            );
        }

        let entry = file(if kit == "react" {
            "main.tsx"
        } else {
            "main.ts"
        });
        let install = entry.find("installPageTools(router)").unwrap_or_else(|| {
            panic!("{kit}: the entry does not install the page tools:\n{entry}")
        });
        let start = entry
            .find("createInertiaApp({")
            .unwrap_or_else(|| panic!("{kit}: the entry does not start Inertia"));
        assert!(
            install < start,
            "{kit}: page tools install after the application starts"
        );
        assert!(
            entry.contains("import { installPageTools } from './lib/agents'"),
            "{kit}: {entry}"
        );
    }
}

/// The kit-agnostic vitest cases every kit's run must pass: the shared
/// browser-call cases, the page tools from Inertia's `navigate` events, and
/// the tool registry.
const CORE_CASES: &[&str] = &[
    "agent_014_server_tool_paths_match_the_shared_cases",
    "agent_014_server_tool_csrf_headers_match_the_shared_cases",
    "agent_014_server_tool_results_match_the_shared_cases",
    "agent_014_page_tools_register_with_their_annotations",
    "agent_014_inertia_visit_replaces_the_page_tools",
    "agent_014_history_back_restores_that_pages_tools",
    "agent_014_invalid_page_tools_register_nothing",
    "agent_014_server_tool_call_carries_session_csrf_and_header",
    "agent_014_write_tool_reloads_the_page_props",
    "agent_014_read_only_tool_does_not_reload",
    "agent_014_unanswered_call_does_not_reload",
    "agent_014_an_aborted_call_rejects_and_does_not_reload",
    "agent_015_duplicate_tool_name_is_refused",
    "agent_015_leaving_tool_holds_until_its_pending_call_settles",
    "agent_015_back_forward_cache_suspends_and_restores_tools",
    "agent_015_a_page_without_webmcp_registers_nothing",
];

/// The kit's own vitest cases, each named `agent_015_<kit>_<case>`.
const KIT_CASES: &[&str] = &[
    "component_tool_registers_while_mounted",
    "component_tool_leaves_on_unmount",
    "form_tool_submits_through_inertia",
    "form_tool_validation_errors_are_an_error_result",
    "destructive_form_tool_fills_and_does_not_submit",
];

/// Whether every vitest case named `name`, by its own title or a `describe`
/// around it, ran and passed, and at least one did.
fn case_passed(report: &serde_json::Value, name: &str) -> bool {
    let mut found = false;
    for file in report["testResults"].as_array().into_iter().flatten() {
        for case in file["assertionResults"].as_array().into_iter().flatten() {
            let titled = case["title"] == name
                || case["ancestorTitles"]
                    .as_array()
                    .is_some_and(|titles| titles.iter().any(|title| title == name));
            if titled {
                if case["status"] != "passed" {
                    return false;
                }
                found = true;
            }
        }
    }
    found
}

/// Scaffold `kit`, install its packages, copy the agent test fixtures and
/// the shared browser-call cases in, and run them with Vitest in jsdom, as
/// `kit_typecheck` runs the stream hooks. A failed run keeps its directory
/// for the logs.
fn kit_tools_in_jsdom(kit: &str) {
    let work = tempfile::Builder::new()
        .prefix(&format!("agents-kit-{kit}-"))
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("create the run directory")
        .keep();
    let report = work.join("vitest-results.json");
    let outcome = (|| -> Result<(), String> {
        let project = scaffold(&work, "agent_tools", kit)?;
        let frontend = project.join("frontend");
        let minutes = |count: u64| Duration::from_secs(60 * count);
        run(
            AsyncCommand::new("npm")
                .args(["install", "--no-audit", "--no-fund"])
                .current_dir(&frontend),
            &work.join("npm-install.log"),
            minutes(10),
        )?;
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser/kit_fixtures");
        let mut copies = vec![
            (fixtures.join("fakes.ts"), "fakes.ts"),
            (fixtures.join("agents-core.test.ts"), "agents-core.test.ts"),
            (fixtures.join(format!("{kit}.test.ts")), "agents.test.ts"),
            (
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../framework/tests/agents/fixtures/browser-calls.json"),
                "browser-calls.json",
            ),
        ];
        if kit == "svelte" {
            copies.push((fixtures.join("AgentProbe.svelte"), "AgentProbe.svelte"));
            copies.push((
                fixtures.join("vitest.svelte.config.ts"),
                "vitest.agents.config.ts",
            ));
        } else {
            copies.push((fixtures.join("vitest.config.ts"), "vitest.agents.config.ts"));
        }
        for (from, to) in copies {
            std::fs::copy(&from, frontend.join(to))
                .map_err(|error| format!("copy {} into the kit: {error}", from.display()))?;
        }
        // `--legacy-peer-deps`: npm 11 fails resolving Vitest's optional
        // peers on an installed tree; the kit already has Vite.
        run(
            AsyncCommand::new("npm")
                .args([
                    "install",
                    "--no-save",
                    "--legacy-peer-deps",
                    "--no-audit",
                    "--no-fund",
                    "vitest@4.1.11",
                    "jsdom@30.1.0",
                ])
                .current_dir(&frontend),
            &work.join("npm-vitest.log"),
            minutes(10),
        )?;
        run(
            AsyncCommand::new("npx")
                .args(["vitest", "run", "--config", "vitest.agents.config.ts"])
                .arg("--reporter=default")
                .arg("--reporter=json")
                .arg(format!("--outputFile.json={}", report.display()))
                .current_dir(&frontend),
            &work.join("vitest.log"),
            minutes(10),
        )
    })();
    if let Err(error) = outcome {
        panic!("{kit}: {error}\nrun directory: {}", work.display());
    }
    let report: serde_json::Value = serde_json::from_str(&read(&report))
        .unwrap_or_else(|error| panic!("{kit}: read the Vitest report: {error}"));
    let kit_cases: Vec<String> = KIT_CASES
        .iter()
        .map(|case| format!("agent_015_{kit}_{case}"))
        .collect();
    let missing: Vec<&str> = CORE_CASES
        .iter()
        .copied()
        .chain(kit_cases.iter().map(String::as_str))
        .filter(|name| !case_passed(&report, name))
        .collect();
    assert!(
        missing.is_empty(),
        "{kit}: these cases did not run and pass: {missing:#?}\nrun directory: {}",
        work.display()
    );
    // A passing run leaves nothing behind: its `node_modules` is large.
    let _ = std::fs::remove_dir_all(&work);
}

/// The React kit's tools in jsdom: `usePageTool` and `useFormTool` over
/// Inertia's own `useForm`, and the shared engine.
#[test]
#[ignore = "AGENT-015 evidence: needs node, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_015_react_tools_in_jsdom() {
    kit_tools_in_jsdom("react");
}

/// The Vue kit's tools in jsdom.
#[test]
#[ignore = "AGENT-015 evidence: needs node, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_015_vue_tools_in_jsdom() {
    kit_tools_in_jsdom("vue");
}

/// The Svelte kit's tools in jsdom.
#[test]
#[ignore = "AGENT-015 evidence: needs node, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_015_svelte_tools_in_jsdom() {
    kit_tools_in_jsdom("svelte");
}

/// The fixture's routes, inserted into the scaffold's `routes!` before the
/// static files: two pages that offer server tools through `with_tools`, a
/// group whose `fixture.help` every fixture page offers, the form page, the
/// tools' own routes and the recorder.
const FIXTURE_ROUTES: &str = r#"    // The AGENT-014 and AGENT-015 browser fixture.
    group!("/agent-fixture", {
        get!("/first", controllers::agent_fixture::first)
            .with_tools(["fixture.bump", "fixture.peek", "fixture.mine"]),
        get!("/second", controllers::agent_fixture::second).with_tools(["fixture.reset"]),
        get!("/form", controllers::agent_fixture::form),
        post!("/bump", controllers::agent_fixture::bump),
        get!("/peek", controllers::agent_fixture::peek),
        post!("/mine", controllers::agent_fixture::mine),
        post!("/reset", controllers::agent_fixture::reset),
        get!("/help", controllers::agent_fixture::help),
        post!("/notes", controllers::agent_fixture::store_note),
        post!("/purge", controllers::agent_fixture::purge),
        get!("/schema", controllers::agent_fixture::schema),
        get!("/requests", controllers::agent_fixture::requests),
    })
    .with_tools(["fixture.help"])
    .middleware(controllers::agent_fixture::Recorder),

"#;

/// The browser cases each kit's run must pass, each named
/// `agent_01x_<kit>_<case>`.
const BROWSER_CASES: &[&str] = &[
    "agent_014_{kit}_page_registers_the_server_tools_the_viewer_may_call",
    "agent_014_{kit}_server_tool_annotations_follow_the_markings",
    "agent_014_{kit}_inertia_visit_replaces_the_page_tools",
    "agent_014_{kit}_history_back_restores_that_pages_tools",
    "agent_014_{kit}_server_tool_call_carries_session_csrf_and_header",
    "agent_014_{kit}_write_tool_reloads_the_page_props",
    "agent_015_{kit}_component_tool_registers_and_leaves_with_its_component",
    "agent_015_{kit}_form_tool_registers_the_generated_schema",
    "agent_015_{kit}_form_tool_fills_and_submits_through_inertia",
    "agent_015_{kit}_form_tool_returns_each_invalid_field",
    "agent_015_{kit}_destructive_form_tool_leaves_the_submit_to_the_user",
];

/// The application process, stopped when dropped.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Replace the one `from` in the file at `path` with `to`.
fn patch(path: &Path, from: &str, to: &str) -> Result<(), String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if source.matches(from).count() != 1 {
        return Err(format!(
            "{} does not hold exactly one `{from}`",
            path.display()
        ));
    }
    std::fs::write(path, source.replacen(from, to, 1)).map_err(|error| error.to_string())
}

/// Add the fixture's controller, routes and pages to the scaffolded kit at
/// `project`, serve its built assets, and regenerate its types, so the
/// form tool's schema comes from `generate-types` as an application's does.
fn add_fixture(project: &Path, kit: &str, work: &Path) -> Result<(), String> {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser/kit_fixtures/app");
    std::fs::copy(
        app.join("agent_fixture.rs"),
        project.join("src/controllers/agent_fixture.rs"),
    )
    .map_err(|error| format!("copy the fixture controller: {error}"))?;
    let controllers = project.join("src/controllers/mod.rs");
    let mut modules = std::fs::read_to_string(&controllers).map_err(|error| error.to_string())?;
    modules.push_str("pub mod agent_fixture;\n");
    std::fs::write(&controllers, modules).map_err(|error| error.to_string())?;
    patch(
        &project.join("src/routes.rs"),
        "    // Static files.",
        &format!("{FIXTURE_ROUTES}    // Static files."),
    )?;
    let variant = match kit {
        "react" => "React",
        "vue" => "Vue",
        _ => "Svelte",
    };
    // Serve the built assets under `APP_ENV=local`, as `kit_browser` does.
    patch(
        &project.join("src/bootstrap.rs"),
        &format!(".frontend(Frontend::{variant})"),
        &format!(".frontend(Frontend::{variant}).development(false)"),
    )?;
    kit_support::patch_local_suprnova(project)?;

    let (ext, widget) = match kit {
        "react" => ("tsx", "AgentFixtureWidget.tsx"),
        "vue" => ("vue", "AgentFixtureWidget.vue"),
        _ => ("svelte", "AgentFixtureWidget.svelte"),
    };
    let pages = project.join("frontend/src/pages/AgentFixture");
    std::fs::create_dir_all(&pages).map_err(|error| error.to_string())?;
    for page in ["First", "Second", "Form"] {
        std::fs::copy(
            app.join(kit).join(format!("{page}.{ext}")),
            pages.join(format!("{page}.{ext}")),
        )
        .map_err(|error| format!("copy the {page} page: {error}"))?;
    }
    std::fs::copy(
        app.join(kit).join(widget),
        project.join("frontend/src/components").join(widget),
    )
    .map_err(|error| format!("copy the widget: {error}"))?;
    run(
        AsyncCommand::new(kit_support::CLI)
            .arg("generate-types")
            .current_dir(project),
        &work.join("generate-types.log"),
        Duration::from_secs(120),
    )
}

/// Wait until `url` answers, or the server exits.
fn ready(server: &mut Server, url: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| error.to_string())?;
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = server.0.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "the application exited before it was ready: {status}"
                ));
            }
            if client
                .get(url)
                .send()
                .await
                .is_ok_and(|reply| reply.status().is_success())
            {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(60) {
                return Err(format!(
                    "the application did not answer {url} within 60 seconds"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

/// Scaffold `kit` with the fixture, build and start it, and run
/// `kits.spec.ts` against it in Chromium with WebMCP. Playwright 1.62.1
/// comes from `tests/kit_browser`'s locked install, and its Chromium must be
/// installed once on the machine, as for `kit_browser`. Set `TMPDIR` to a
/// short directory: Chromium refuses a long socket path.
fn kit_tools_in_a_browser(kit: &str) {
    let work = tempfile::Builder::new()
        .prefix(&format!("agents-browser-{kit}-"))
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .expect("create the run directory")
        .keep();
    let report = work.join("playwright-report.json");
    let server_log = work.join("server.log");
    let minutes = |count: u64| Duration::from_secs(60 * count);
    let outcome = (|| -> Result<(), String> {
        let name = "agent_browser_app";
        let project = scaffold(&work, name, kit)?;
        add_fixture(&project, kit, &work)?;
        let frontend = project.join("frontend");
        run(
            AsyncCommand::new("npm")
                .args(["install", "--no-audit", "--no-fund"])
                .current_dir(&frontend),
            &work.join("npm-install.log"),
            minutes(10),
        )?;
        run(
            AsyncCommand::new("npm")
                .args(["run", "build"])
                .current_dir(&frontend),
            &work.join("npm-build.log"),
            minutes(10),
        )?;
        // Reuse the outer build's dependency artifacts.
        let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .parent()
            .ok_or("the cargo target directory")?
            .to_path_buf();
        run(
            AsyncCommand::new(env!("CARGO"))
                .args(["build", "--bin", name])
                .current_dir(&project)
                .env("CARGO_TARGET_DIR", &target),
            &work.join("cargo-build.log"),
            minutes(40),
        )?;

        let suite_modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/kit_browser");
        if !suite_modules
            .join("node_modules/@playwright/test/cli.js")
            .exists()
        {
            run(
                AsyncCommand::new("npm")
                    .arg("ci")
                    .current_dir(&suite_modules),
                &work.join("playwright-npm.log"),
                minutes(5),
            )?;
        }
        let suite = work.join("suite");
        std::fs::create_dir_all(&suite).map_err(|error| error.to_string())?;
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agents_browser");
        for file in ["kits.spec.ts", "kits.playwright.config.ts"] {
            std::fs::copy(source.join(file), suite.join(file))
                .map_err(|error| format!("copy {file}: {error}"))?;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            suite_modules.join("node_modules"),
            suite.join("node_modules"),
        )
        .map_err(|error| error.to_string())?;

        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map_err(|error| error.to_string())?
            .port();
        let url = format!("http://127.0.0.1:{port}");
        let log = std::fs::File::create(&server_log).map_err(|error| error.to_string())?;
        let mut server = Server(
            std::process::Command::new(target.join("debug").join(name))
                .arg("web:run")
                .current_dir(&project)
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
                .stdout(log.try_clone().map_err(|error| error.to_string())?)
                .stderr(log)
                .spawn()
                .map_err(|error| format!("start the application: {error}"))?,
        );
        ready(&mut server, &format!("{url}/agent-fixture/first"))?;
        run(
            AsyncCommand::new("node")
                .arg(suite.join("node_modules/@playwright/test/cli.js"))
                .args(["test", "--config", "kits.playwright.config.ts"])
                .current_dir(&suite)
                .env("KIT_BASE_URL", &url)
                .env("KIT_NAME", kit)
                .env("KIT_AGENTS_REPORT", &report)
                .env("KIT_AGENTS_RESULTS", work.join("results")),
            &work.join("playwright.log"),
            minutes(20),
        )
    })();
    if let Err(error) = outcome {
        panic!(
            "{kit}: {error}\nserver log:\n{}\nrun directory: {}",
            read(&server_log),
            work.display()
        );
    }
    let report: serde_json::Value = serde_json::from_str(&read(&report))
        .unwrap_or_else(|error| panic!("{kit}: read the Playwright report: {error}"));
    let mut passed = Vec::new();
    collect_passed(&report["suites"], &mut passed);
    let missing: Vec<String> = BROWSER_CASES
        .iter()
        .map(|case| case.replace("{kit}", kit))
        .filter(|case| !passed.contains(case))
        .collect();
    assert!(
        missing.is_empty(),
        "{kit}: these cases did not run and pass: {missing:#?}\nrun directory: {}",
        work.display()
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// The titles of the Playwright report's specs whose every result passed.
fn collect_passed(suites: &serde_json::Value, passed: &mut Vec<String>) {
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
                passed.push(title.to_string());
            }
        }
        collect_passed(&suite["suites"], passed);
    }
}

/// The React kit's page tools, component tools and form tools in Chromium.
#[test]
#[ignore = "AGENT-014 and AGENT-015 evidence: needs Playwright's Chromium 1.62.1, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_014_and_015_react_tools_in_a_browser() {
    kit_tools_in_a_browser("react");
}

/// The Vue kit's tools in Chromium.
#[test]
#[ignore = "AGENT-014 and AGENT-015 evidence: needs Playwright's Chromium 1.62.1, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_014_and_015_vue_tools_in_a_browser() {
    kit_tools_in_a_browser("vue");
}

/// The Svelte kit's tools in Chromium.
#[test]
#[ignore = "AGENT-014 and AGENT-015 evidence: needs Playwright's Chromium 1.62.1, npm and the network; the agents-browser mechanism runs --run-ignored only"]
fn agent_014_and_015_svelte_tools_in_a_browser() {
    kit_tools_in_a_browser("svelte");
}
