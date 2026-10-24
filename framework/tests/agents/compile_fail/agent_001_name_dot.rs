use suprnova::{handler, tool, Response};

#[tool(name = "bad.dot", description = "A tool.", callers = [public])]
#[handler]
async fn invalid() -> Response { suprnova::json_response!({}) }

fn main() {}
