// AGENT-001: a tool name is 1 to 128 ASCII letters, digits, `_`, `-` and `.`.
use suprnova::{Response, handler, json_response, tool};

#[tool(name = "posts create", description = "A space in the name.", callers = [public])]
#[handler]
pub async fn spaced() -> Response {
    json_response!({})
}

#[tool(
    name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    description = "129 characters.",
    callers = [public]
)]
#[handler]
pub async fn long() -> Response {
    json_response!({})
}

fn main() {}
