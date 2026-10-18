// AGENT-001: two tools with one name in a crate do not build, wherever they
// are declared.
use suprnova::{Response, handler, json_response, tool};

mod first {
    use super::*;

    #[tool(name = "posts.create", description = "The first.", callers = [public])]
    #[handler]
    pub async fn create() -> Response {
        json_response!({})
    }
}

mod second {
    use super::*;

    #[tool(name = "posts.create", description = "The second.", callers = [public])]
    #[handler]
    pub async fn create() -> Response {
        json_response!({})
    }
}

pub struct Posts;

impl Posts {
    #[tool(name = "posts.create", description = "The third, in an impl block.", callers = [public])]
    #[handler(Self = Posts)]
    pub async fn create() -> Response {
        json_response!({})
    }
}

fn main() {}
