//! The page exists at the starter location; only the manifest is wrong.

use suprnova::{Request, Response, inertia_response};

pub async fn home(req: Request) -> Response {
    inertia_response!(&req, "Home", { "title": "Home" })
}
