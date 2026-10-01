//! Both pages exist at the paths the manifest's lookup resolves.

use suprnova::{Request, Response, inertia_response};

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Tramits/Index", { "title": "Tramits" })
}

pub async fn create(req: Request) -> Response {
    inertia_response!(&req, "Tramits/BaixaMatricula/Create", { "title": "Baixa" })
}
