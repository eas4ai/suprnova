// AGENT-001: a tool without a description, or with an empty one, does not
// build.
use suprnova::{Response, handler, json_response, tool};

#[tool(name = "silent", callers = [public])]
#[handler]
pub async fn silent() -> Response {
    json_response!({})
}

#[tool(name = "blank", description = "  ", callers = [public])]
#[handler]
pub async fn blank() -> Response {
    json_response!({})
}

fn main() {}
