// AGENT-002: a tool that accepts `public` callers cannot declare scopes: a
// guest calls it without a token, so a scope would protect nothing.
use suprnova::{Response, handler, json_response, tool};

suprnova::tool_scopes! {
    pub mod scope {
        POSTS_READ = "posts:read" => "Read your posts";
    }
}

#[tool(
    name = "posts.feed",
    description = "List posts.",
    callers = [public, user],
    scopes = [scope::POSTS_READ]
)]
#[handler]
pub async fn feed() -> Response {
    json_response!({})
}

fn main() {}
