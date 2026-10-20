//! Compiler diagnostics must reach cargo and editors without rejecting tools.

#[test]
fn agent_001_webmcp_budget_warnings_reach_cargo_check() {
    use std::{fs, path::Path, process::Command};

    let fixture = tempfile::tempdir().expect("fixture directory");
    let framework = Path::new(env!("CARGO_MANIFEST_DIR")).join("../framework");
    let manifest = format!(
        "[package]\nname = \"tool-warning-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nsuprnova = {{ path = {:?} }}\n",
        framework,
    );
    fs::write(fixture.path().join("Cargo.toml"), manifest).expect("fixture manifest");
    fs::copy(
        framework.join("../Cargo.lock"),
        fixture.path().join("Cargo.lock"),
    )
    .expect("the workspace's pinned dependencies");
    fs::create_dir(fixture.path().join("src")).expect("fixture source directory");
    fs::create_dir_all(fixture.path().join("templates/live/agents")).expect("fixture templates");
    fs::write(
        fixture.path().join("templates/live/agents/tool.html"),
        "<p>{{ count }}</p>",
    )
    .expect("fixture view");
    let source = format!(
        r#"
use suprnova::{{handler, tool, Response}};
#[tool(name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", description = "{}", callers = [public])]
#[handler]
async fn over_budget() -> Response {{ suprnova::json_response!({{}}) }}
#[tool(name = "within_budget", description = "A short description.", callers = [public])]
#[handler]
async fn within_budget() -> Response {{ suprnova::json_response!({{}}) }}
#[derive(suprnova::live::LiveComponent)]
#[live(name = "test.warnings", view = "live/agents/tool.html")]
pub struct WarningCounter {{ count: u64 }}
#[suprnova::live::live]
impl WarningCounter {{
    #[tool(name = "search_counter", description = "Search the counter.", callers = [public])]
    #[action]
    pub fn search(&mut self, query: String) {{ let _ = query; }}
    #[tool(name = "describe_counter", description = "Describe the counter.", callers = [public], arguments(query = "The label to find."))]
    #[action]
    pub fn described(&mut self, query: String) {{ let _ = query; }}
}}
"#,
        "d".repeat(501),
    );
    fs::write(fixture.path().join("src/lib.rs"), source).expect("fixture tools");
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("the test target directory");
    let output = Command::new(env!("CARGO"))
        .args(["check", "--offline", "--manifest-path"])
        .arg(fixture.path().join("Cargo.toml"))
        .arg("--target-dir")
        .arg(target)
        .output()
        .expect("cargo check the tools");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    for expected in [
        "tool `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`: the name has 31 characters; WebMCP's budget is 30",
        "tool `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`: the description has 501 characters; WebMCP's budget is 500",
        "tool `search_counter`: input field `query` has no description; WebMCP requires a description",
    ] {
        assert!(
            stderr
                .lines()
                .any(|line| line.starts_with("warning:") && line.contains(expected)),
            "{stderr}"
        );
    }
    assert!(!stderr.contains("tool `within_budget`:"), "{stderr}");
    assert!(!stderr.contains("tool `describe_counter`:"), "{stderr}");
}
