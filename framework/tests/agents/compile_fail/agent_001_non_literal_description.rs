// AGENT-001: names and descriptions are fixed at build time, never taken
// from data, so they are string literals.
use suprnova::{Response, handler, json_response, tool};

const DESCRIPTION: &str = "From a constant.";

#[tool(name = "computed", description = DESCRIPTION, callers = [public])]
#[handler]
pub async fn computed() -> Response {
    json_response!({})
}

fn main() {}
