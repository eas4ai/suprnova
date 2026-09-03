//! The Live document routes: an authenticated dashboard with three islands
//! and a public page with one public seed.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, StatusCode};

use super::{DashboardMounts, PublicMounts};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/dashboard.html")]
struct DashboardView<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
    uploader: &'a TrustedHtml,
    feed: &'a TrustedHtml,
}

#[suprnova::view(path = "live/public.html")]
struct PublicView<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

fn parameters() -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::new())
}

fn view(name: &str) -> Result<ViewName, FrameworkError> {
    ViewName::parse(name).map_err(|_| FrameworkError::internal("Live view identity"))
}

fn intent() -> Result<DocumentResponseIntent, FrameworkError> {
    DocumentResponseIntent::html(StatusCode::OK)
        .map_err(|_| FrameworkError::internal("Live document response intent"))
}

fn failed(error: FrameworkError) -> HttpResponse {
    // The detail goes to the log only; a visitor sees a closed failure.
    tracing::warn!(error = %error, "Live document failed");
    HttpResponse::text("Live document failed").status(500)
}

/// `GET /live`: identity-bound counter, avatar uploader, and activity feed.
pub async fn dashboard(request: Request, mounts: &DashboardMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(&mounts.counter, parameters(), MountFlags::empty())
            .await?;
        let uploader = document
            .mount(&mounts.uploader, parameters(), MountFlags::empty())
            .await?;
        let feed = document
            .mount(&mounts.feed, parameters(), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                view("live/dashboard.html")?,
                &DashboardView {
                    bootstrap: bootstrap.html(),
                    counter: counter.html(),
                    uploader: uploader.html(),
                    feed: feed.html(),
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}

/// `GET /live/public`: one public seed any visitor can render.
pub async fn public(request: Request, mounts: &PublicMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(&mounts.counter, parameters(), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                view("live/public.html")?,
                &PublicView {
                    bootstrap: bootstrap.html(),
                    counter: counter.html(),
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}
