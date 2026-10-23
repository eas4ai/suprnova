//! Pages at the starter location, with two of the four extensions.

use suprnova::{Request, Response, inertia_response};

pub async fn home(req: Request) -> Response {
    inertia_response!(&req, "Home", { "title": "Home" })
}

pub async fn users(req: Request) -> Response {
    inertia_response!(&req, "Users/Index", { "users": [] })
}
