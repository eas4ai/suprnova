//! BIND-003: a `#[handler]` function inside an `impl` block names its type,
//! `#[handler(Self = Posts)]`, so its record can name it. Without it the
//! build fails and says so.

use suprnova::{Response, handler};

pub struct Posts;

impl Posts {
    #[handler]
    pub async fn show(id: i64) -> Response {
        suprnova::http::text(id.to_string())
    }
}

fn main() {}
