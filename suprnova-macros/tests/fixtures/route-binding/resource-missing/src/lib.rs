//! BIND-011: a function-form resource whose module lacks a selected
//! action's function fails to compile. `posts` has no `destroy`.

use suprnova::{Response, handler, resource};

pub mod posts {
    use super::*;

    #[handler]
    pub async fn index() -> Response {
        suprnova::http::text("index")
    }
    #[handler]
    pub async fn create() -> Response {
        suprnova::http::text("create")
    }
    #[handler]
    pub async fn store() -> Response {
        suprnova::http::text("store")
    }
    #[handler]
    pub async fn show() -> Response {
        suprnova::http::text("show")
    }
    #[handler]
    pub async fn edit() -> Response {
        suprnova::http::text("edit")
    }
    #[handler]
    pub async fn update() -> Response {
        suprnova::http::text("update")
    }
}

pub fn router() -> suprnova::Router {
    resource!("posts", posts).register(suprnova::Router::new())
}
