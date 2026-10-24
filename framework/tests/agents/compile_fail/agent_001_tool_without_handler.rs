// AGENT-001: `#[tool]` marks a routed `#[handler]`; on a bare function it
// does not build.
use suprnova::{Response, json_response, tool};

#[tool(name = "lonely", description = "No handler under it.", callers = [public])]
pub async fn lonely() -> Response {
    json_response!({})
}

fn main() {}
