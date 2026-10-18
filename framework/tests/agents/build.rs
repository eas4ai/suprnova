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
