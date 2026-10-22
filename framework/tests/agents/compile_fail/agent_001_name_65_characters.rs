use suprnova::{handler, tool, Response};

#[tool(name = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", description = "A tool.", callers = [public])]
#[handler]
async fn invalid() -> Response { suprnova::json_response!({}) }

fn main() {}
