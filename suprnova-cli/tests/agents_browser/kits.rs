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
