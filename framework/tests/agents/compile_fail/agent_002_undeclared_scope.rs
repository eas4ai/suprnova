// AGENT-002: a tool naming a scope no `tool_scopes!` declared does not build.
use suprnova::{Response, handler, json_response, tool};

suprnova::tool_scopes! {
    pub mod scope {
        POSTS_READ = "posts:read" => "Read your posts";
    }
}

#[tool(
    name = "posts.purge",
    description = "Delete every post.",
    callers = [user],
    scopes = [scope::POSTS_PURGE]
)]
#[handler]
pub async fn purge() -> Response {
    json_response!({})
}

suprnova::tool_scopes! {
    pub mod bad {
        SPACED = "posts read" => "A space is not allowed in a scope name";
    }
}

fn main() {}
