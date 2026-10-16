//! BIND-011: an action `only` leaves out needs no function. This crate
//! must compile.

use suprnova::{Response, handler, resource};

pub mod posts {
    use super::*;

    #[handler]
    pub async fn index() -> Response {
        suprnova::http::text("index")
    }
}

pub fn router() -> suprnova::Router {
    resource!("posts", posts, only = [index]).register(suprnova::Router::new())
}
