//! The rules a tool's marking must follow fail the build when broken
//! (AGENT-001, AGENT-002). Only a real build proves a build failure, so each
//! case compiles under `trybuild` and its error is compared with the
//! recorded one.

#[test]
fn agent_001_invalid_tool_markings_fail_to_build() {
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/agent_001_*.rs");
}

#[test]
fn agent_002_undeclared_scope_fails_to_build() {
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/agent_002_*.rs");
}

/// The templates the Live cases' components render, where the build
/// `trybuild` runs looks for them: its own crate's `templates/`.
fn install_live_templates() {
    let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR sits inside the target directory")
        .to_path_buf();
    let directory = target.join("tests/trybuild/suprnova/templates/live/agents");
    std::fs::create_dir_all(&directory).expect("create the trybuild template directory");
    std::fs::write(directory.join("tool.html"), "<p>{{ count }}</p>\n")
        .expect("write the trybuild template");
}

#[test]
fn agent_001_live_tool_markings_fail_to_build() {
    install_live_templates();
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/live/agent_001_*.rs");
}

#[test]
fn agent_002_live_tool_with_undeclared_scope_fails_to_build() {
    install_live_templates();
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/live/agent_002_*.rs");
}
