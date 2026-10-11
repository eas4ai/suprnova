//! No `edit.page.ts` exists next to `create.page.ts`.

use suprnova::{Request, Response, inertia_response};

pub async fn edit(req: Request) -> Response {
    inertia_response!(&req, "Tramits/BaixaMatricula/Edit", { "title": "Edit" })
}

fn main() {}
