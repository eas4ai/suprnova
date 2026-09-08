//! The Live document routes: an authenticated dashboard with three islands,
//! a public page with one public seed, an ORM-backed public todo listing,
//! and the signed-in visitor's own account page.
//!
//! The last two mount no island. They are still document routes of this
//! application's Live surface, and they exist to exercise the two
//! RenderCache shapes the island documents cannot: a shared representation
//! whose content comes from the ORM (so an ordinary model write is what
//! invalidates it), and a representation stored once per principal.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName, ViewTemplate};
use suprnova::{Auth, FrameworkError, HttpResponse, Model, Request, Response, StatusCode};

use super::{DashboardMounts, PublicMounts};
use crate::models::todos::Todo;
use crate::models::users::User;

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

#[suprnova::view(path = "live/todos.html")]
struct TodosView {
    count: usize,
    titles: Vec<String>,
}

#[suprnova::view(path = "live/me.html")]
struct MeView {
    name: String,
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

/// Renders one checked template into an HTML response.
///
/// The two island-free pages take this path rather than
/// `LiveDocument::render`: with no mount to describe there is no document
/// metadata to validate, and no Live bootstrap worth emitting for a page
/// that has nothing to bootstrap.
fn html(view: &impl ViewTemplate) -> Result<HttpResponse, FrameworkError> {
    let mut body = String::new();
    view.render_view(&mut body)
        .map_err(|_| FrameworkError::internal("Live page template"))?;
    Ok(HttpResponse::html(body))
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

/// `GET /live/todos`: every todo in the database, listed through the ORM.
///
/// The read is an ordinary `Todo::all()`, so the RenderCache collector
/// records the `todos` table as a dependency of the render and any model
/// write to that table advances its generation. Nothing here reads the
/// session, the signed-in visitor, or the locale, which is what lets the
/// route stay a shared representation.
pub async fn todos(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let titles: Vec<String> = Todo::all()
            .await?
            .into_vec()
            .into_iter()
            .map(|todo| todo.title)
            .collect();
        html(&TodosView {
            count: titles.len(),
            titles,
        })
    }
    .await;
    result.map_err(failed)
}

/// `GET /live/me`: the signed-in visitor's own account page.
///
/// The route's own `AuthMiddleware::redirect_to("/login")` is what turns an
/// anonymous visit into a redirect, exactly as on the dashboard, so this
/// handler only ever runs for a resolved principal; the `ok_or_else` below
/// is a contract check, not a gate.
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
