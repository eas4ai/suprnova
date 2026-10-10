//! BIND-003: `#[handler(Self = Type)]` names the type of the `impl` block
//! the handler is in. Naming another type fails the build, so a record
//! can never key one type's function with another's arguments.

use suprnova::{Response, handler};

pub struct Posts;
pub struct Comments;

impl Comments {
    pub async fn show(_id: i64) -> Response {
        suprnova::http::text("comment")
    }
}

impl Posts {
    #[handler(Self = Comments)]
    pub async fn show(id: i64) -> Response {
        suprnova::http::text(id.to_string())
    }
}

fn main() {}
