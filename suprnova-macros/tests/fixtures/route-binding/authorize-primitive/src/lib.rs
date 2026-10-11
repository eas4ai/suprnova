//! BIND-003 and BIND-015: a primitive `#[authorize]` target keeps
//! compiling, beside a bound one. This crate must compile.

use suprnova::{Response, handler, model};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
}

#[handler]
#[suprnova::authorize("show", id)]
pub async fn show(id: i64) -> Response {
    suprnova::http::text(id.to_string())
}

#[handler]
#[suprnova::authorize("view", post)]
pub async fn view(post: Post) -> Response {
    suprnova::http::text(post.title)
}
