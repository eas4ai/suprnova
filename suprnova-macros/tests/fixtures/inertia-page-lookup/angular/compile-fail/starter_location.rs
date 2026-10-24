//! `frontend/src/pages/Home.svelte` exists, but the manifest moved the
//! lookup, so the starter location no longer counts.

use suprnova::{Request, Response, inertia_response};

pub async fn home(req: Request) -> Response {
    inertia_response!(&req, "Home", { "title": "Home" })
}

fn main() {}
