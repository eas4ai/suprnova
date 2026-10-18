// AGENT-001: the input schema is the type the handler validates, so
// `input = T` must name a type of one of the handler's arguments.
use suprnova::{Response, handler, json_response, tool, tool_input};

#[tool_input]
pub struct CreatePost {
    pub title: String,
}

#[tool_input]
pub struct RenamePost {
    pub name: String,
}

#[tool(name = "posts.create", description = "Create a post.", callers = [public], input = RenamePost)]
#[handler]
pub async fn create(form: CreatePost) -> Response {
    json_response!({ "title": form.title })
}

#[tool(name = "posts.raw", description = "Read the body itself.", callers = [public], input = CreatePost)]
#[handler]
pub async fn raw(request: suprnova::Request) -> Response {
    let _ = request;
    json_response!({})
}

mod other {
    #[suprnova::tool_input]
    pub struct CreatePost {
        pub body: String,
    }
}

#[tool(name = "posts.other", description = "Same name, other type.", callers = [public], input = CreatePost)]
#[handler]
pub async fn other(form: other::CreatePost) -> Response {
    json_response!({ "body": form.body })
}

fn main() {}
