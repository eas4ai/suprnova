//! The welcome page.

use suprnova::live::LiveDocument;
use suprnova::{__, FrameworkError, HttpResponse, Request, Response, handler};

use crate::views::{self, APP_NAME, Frame, HomePage, Nav};

/// `GET /`: a Live document that mounts no island.
#[handler]
pub async fn index(req: Request) -> Response {
    Ok(page(&req).await?)
}

async fn page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let frame = Frame::live(&mut document, "Home", Nav::None).await?;
    let page = HomePage {
        frame,
        // `welcome = Welcome to { $app }!` in `lang/en/app.ftl`.
        welcome: __!("welcome", app: APP_NAME),
    };
    views::render(document, "live/pages/home.html", &page)
}
