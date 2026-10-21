//! The manual's account of agents (AGENT-019): the `agents.md` chapter and
//! its link from the index, the parity rows for the MCP server and Passport,
//! and a row in the environment variable table for every variable the
//! agents code reads.

use std::path::{Path, PathBuf};

/// The repository root: the framework crate's parent.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the framework crate sits in the repository root")
        .to_path_buf()
}

/// The manual chapter `name`, or an empty text when it does not exist, so
/// a missing chapter fails the assertion that names what is missing.
fn chapter(name: &str) -> String {
    std::fs::read_to_string(root().join("manual").join(name)).unwrap_or_default()
}

/// Every phrase of `phrases` that `text` lacks.
fn missing<'a>(text: &str, phrases: &[&'a str]) -> Vec<&'a str> {
    phrases
        .iter()
        .copied()
        .filter(|phrase| !text.contains(phrase))
        .collect()
}

/// The variables the agents code reads, each of which needs a row in
/// `manual/env-vars.md`. A new read in the agents code fails
/// `agent_019_every_variable_the_agents_code_reads_is_listed` until it is
/// added here, and then this test until the table has its row.
const VARIABLES: &[&str] = &[
    "AGENT_AUTH_KEY_ID",
    "AGENT_AUTH_PRIVATE_KEY",
    "AGENT_AUTH_PREVIOUS_KEY_ID",
    "AGENT_AUTH_PREVIOUS_PRIVATE_KEY",
    "AGENT_AUTH_PREVIOUS_ROTATED_AT",
    "WEBMCP_ORIGIN_TRIAL_TOKEN",
    "APP_URL",
];

/// The source the agents feature added: the framework's agents module and
/// Live tools, and the two Magnetar plugins.
const AGENTS_SOURCE: &[&str] = &[
    "framework/src/agents",
    "framework/src/live/agent.rs",
    "crates/suprnova-magnetar/src/agent_auth",
    "crates/suprnova-magnetar/src/authorization_server",
];

#[test]
fn agent_019_the_chapter_explains_every_required_part() {
    let text = chapter("agents.md");
    let parts: &[(&str, &[&str])] = &[
        (
            "tools",
            &[
                "## Tools",
                "#[tool(",
                "tool_scopes!",
                "callers = [",
                "read_only",
                "caller_limit",
            ],
        ),
        (
            "the MCP endpoint",
            &["## The MCP endpoint", "/mcp", ".agents("],
        ),
        (
            "the three caller kinds",
            &["## Callers", "`public`", "`user`", "`agent`"],
        ),
        (
            "the authorization server plugin",
            &["OAuthServer", "ConnectedAgents"],
        ),
        ("outside issuers", &["OutsideIssuer"]),
        (
            "the agent auth plugin",
            &[
                "AgentAuth",
                "/auth.md",
                "claim",
                "agents:revoke-all",
                "agents:prune",
                "agents:key",
            ],
        ),
        (
            "WebMCP in the browser for Live",
            &["document.modelContext", "### Live actions as tools"],
        ),
        ("page tools", &["share_tools", "with_tools", "PageTools"]),
        (
            "the Inertia kits",
            &[
                "installPageTools",
                "usePageTool",
                "useFormTool",
                "generate-types",
                "React",
                "Vue",
                "Svelte",
            ],
        ),
        (
            "WebAssembly front ends",
            &["suprnova-webmcp", "WebAssembly"],
        ),
        (
            "headless Live tools",
            &["## Headless Live tools", "headless"],
        ),
        ("testing", &["## Testing", "McpTestClient"]),
        (
            "the chapter shape",
            &["### Why Suprnova diverges", "## Next", "```rust"],
        ),
    ];
    let gaps: Vec<String> = parts
        .iter()
        .filter_map(|(part, phrases)| {
            let lacking = missing(&text, phrases);
            (!lacking.is_empty()).then(|| format!("{part}: {lacking:?}"))
        })
        .collect();
    assert!(
        gaps.is_empty(),
        "manual/agents.md lacks these parts: {gaps:#?}"
    );
}

#[test]
fn agent_019_the_chapter_states_the_rulings_where_they_belong() {
    let text = chapter("agents.md");
    let rulings = [
        // An outside issuer finds the user by `sub` only where the issuer
        // keeps one `sub` per user across clients; Entra does not.
        "id_claim(\"oid\")",
        "https://login.microsoftonline.com/",
        "email_verified",
        "verified_by(",
        // What an unclaimed agent can call.
        "is_claimed()",
        "post_claim_scopes",
        // The claim event is delivered at least once.
        "AgentClaimed",
        "event_id",
        "--retry-failed",
        // Keys and operations.
        "AGENT_AUTH_PREVIOUS_",
        "agents:revoke-all --force",
        "CORS",
        // The OAuth server's issuer.
        "APP_URL",
        // Agent auth matches emails without case.
        "LOWER(email)",
        // RenderCache and tools.
        "RenderCache",
    ];
    let lacking = missing(&text, &rulings);
    assert!(
        lacking.is_empty(),
        "manual/agents.md does not state: {lacking:?}"
    );
}

#[test]
fn agent_019_the_documentation_index_links_the_chapter() {
    let index = chapter("documentation.md");
    assert!(
        index.contains("[Agents](agents.md)"),
        "manual/documentation.md does not link manual/agents.md"
    );
}

#[test]
fn agent_019_no_parity_row_says_mcp_is_by_design_no_or_passport_not_built() {
    let parity = chapter("parity.md");
    let rows: Vec<&str> = parity
        .lines()
        .filter(|line| line.starts_with('|'))
        .collect();
    let mcp: Vec<&str> = rows
        .iter()
        .copied()
        .filter(|row| row.contains("MCP"))
        .collect();
    let passport: Vec<&str> = rows
        .iter()
        .copied()
        .filter(|row| row.contains("Passport"))
        .collect();
    assert!(!mcp.is_empty(), "manual/parity.md has no MCP row");
    assert!(!passport.is_empty(), "manual/parity.md has no Passport row");
    let stale: Vec<&str> = mcp
        .iter()
        .copied()
        .filter(|row| row.contains("by design no"))
        .chain(
            passport
                .iter()
                .copied()
                .filter(|row| row.contains("not built") || row.contains("Hydra")),
        )
        .collect();
    assert!(
        stale.is_empty(),
        "these parity rows still say the MCP server or Passport is missing: {stale:#?}"
    );
}

#[test]
fn agent_019_every_listed_variable_has_an_environment_row() {
    let table = chapter("env-vars.md");
    let lacking: Vec<&str> = VARIABLES
        .iter()
        .copied()
        .filter(|name| {
            !table
                .lines()
                .any(|line| line.starts_with(&format!("| `{name}` |")))
        })
        .collect();
    assert!(
        lacking.is_empty(),
        "manual/env-vars.md has no row for: {lacking:?}"
    );
}

/// The `.rs` files under `path`, or `path` itself when it is a file.
fn rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let entry = entry.path();
        if entry.is_dir() {
            rust_files(&entry, files);
        } else if entry.extension().is_some_and(|extension| extension == "rs") {
            files.push(entry);
        }
    }
}

/// The first double-quoted text on `line`.
fn quoted(line: &str) -> Option<&str> {
    let start = line.find('"')? + 1;
    let length = line[start..].find('"')?;
    Some(&line[start..start + length])
}

/// Whether `name` is written as an environment variable: an upper-case
/// letter, then upper-case letters, digits and underscores, with at least
/// one underscore.
fn variable_shaped(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.contains('_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[test]
fn agent_019_every_variable_the_agents_code_reads_is_listed() {
    let mut files = Vec::new();
    for path in AGENTS_SOURCE {
        rust_files(&root().join(path), &mut files);
    }
    assert!(!files.is_empty(), "the agents source was not found");
    let mut read = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("a source file reads");
        for line in source.lines() {
            // A constant naming a variable, which the code passes to an
            // environment read, or a read that names its variable inline.
            let names_variable = line.contains("_VARIABLE: &str =")
                || ["env::var(", "env_optional", "env_required", "env_strict"]
                    .iter()
                    .any(|read| line.contains(read));
            if !names_variable {
                continue;
            }
            if let Some(name) = quoted(line).filter(|name| variable_shaped(name)) {
                read.push((name.to_string(), file.clone()));
            }
        }
    }
    assert!(
        read.iter().any(|(name, _)| name == "AGENT_AUTH_KEY_ID"),
        "the scan found no variable the agents code reads: {read:?}"
    );
    let unlisted: Vec<_> = read
        .iter()
        .filter(|(name, _)| !VARIABLES.contains(&name.as_str()))
        .collect();
    assert!(
        unlisted.is_empty(),
        "the agents code reads variables this test does not list: {unlisted:?}"
    );
}

#[test]
fn agent_019_the_security_headers_and_upgrade_chapters_say_what_changed() {
    let headers = chapter("security-headers.md");
    let lacking = missing(
        &headers,
        &["WebMCP", "tools=", "allow=\"tools\"", "Origin-Trial"],
    );
    assert!(
        lacking.is_empty(),
        "manual/security-headers.md does not say: {lacking:?}"
    );
    let upgrade = chapter("upgrade-4.4.md");
    assert!(
        upgrade.contains("(agents.md"),
        "manual/upgrade-4.4.md does not point at the agents chapter"
    );
}
