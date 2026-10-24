//! The server tools a Live page offers, in Chromium with WebMCP (AGENT-014):
//! `server-tools.spec.ts` against `agents_live_host`'s `/server-tools`.

use super::spec;

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_page_registers_the_server_tools_the_viewer_may_call() {
    spec("agent_014_live_page_registers_the_server_tools_the_viewer_may_call");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_page_tools_leave_on_navigation() {
    spec("agent_014_live_page_tools_leave_on_navigation");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_server_tool_call_carries_session_csrf_and_header() {
    spec("agent_014_live_server_tool_call_carries_session_csrf_and_header");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_write_tool_rerenders_the_mounted_islands() {
    spec("agent_014_live_write_tool_rerenders_the_mounted_islands");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_read_only_tool_does_not_rerender() {
    spec("agent_014_live_read_only_tool_does_not_rerender");
}

#[test]
#[ignore = "needs Playwright's Chromium 1.62.1; run with --run-ignored only"]
fn agent_014_live_server_tool_annotations_follow_the_markings() {
    spec("agent_014_live_server_tool_annotations_follow_the_markings");
}
