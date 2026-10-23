// AGENT-001: a budget opt-out accepts only `budget = ignore`.
use suprnova::{Response, handler, json_response, tool};

#[tool(name = "quiet_budget", description = "Search.", callers = [public], budget = quiet)]
#[handler]
pub async fn quiet_budget() -> Response {
    json_response!({})
}

#[tool(name = "quoted_budget", description = "Search.", callers = [public], budget = "ignore")]
#[handler]
pub async fn quoted_budget() -> Response {
    json_response!({})
}

#[tool(name = "bare_budget", description = "Search.", callers = [public], budget)]
#[handler]
pub async fn bare_budget() -> Response {
    json_response!({})
}

fn main() {}
