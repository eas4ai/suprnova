//! No page named `Missing` exists under `frontend/src/pages`.

use suprnova::{Request, Response, inertia_response};

pub async fn missing(req: Request) -> Response {
    inertia_response!(&req, "Missing", { "title": "Missing" })
}

fn main() {}
