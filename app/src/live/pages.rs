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

use serde::{Deserialize, Serialize};
use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, MountFlags};
use suprnova::session::session_mut;
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName, ViewTemplate};
use suprnova::{Auth, FrameworkError, HttpResponse, Model, Request, Response, StatusCode};

use super::{
    DashboardMounts, DataDisplayMounts, FEEDBACK_PATH, FeedbackMounts, FormsMounts,
    NavigationMounts, OverlaysMounts, PublicMounts,
};
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

#[suprnova::view(path = "live/forms.html")]
struct FormsView<'a> {
    bootstrap: &'a TrustedHtml,
    gallery: &'a TrustedHtml,
}

#[suprnova::view(path = "live/overlays.html")]
struct OverlaysView<'a> {
    bootstrap: &'a TrustedHtml,
    gallery: &'a TrustedHtml,
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
/// The form gallery: the suprnova-ui base is opted in and every
/// presentational form component is mounted once (Cairn FORM-001).
pub async fn forms(request: Request, mounts: &FormsMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let gallery = document
            .mount(&mounts.gallery, parameters(), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        document
            .render(
                view("live/forms.html")?,
                &FormsView {
                    bootstrap: bootstrap.html(),
                    gallery: gallery.html(),
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}

/// The overlay gallery: the suprnova-ui base is opted in and every overlay
/// and disclosure component is mounted once (Cairn OVL-001).
pub async fn overlays(request: Request, mounts: &OverlaysMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let gallery = document
            .mount(&mounts.gallery, parameters(), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        document
            .render(
                view("live/overlays.html")?,
                &OverlaysView {
                    bootstrap: bootstrap.html(),
                    gallery: gallery.html(),
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}

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

/// One message the flash region renders after a redirect (FDB-004): the
/// outcome the previous request left in the session, consumed on this read.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FlashMessage {
    /// info, success, warning or error.
    pub variant: String,
    /// The message text.
    pub text: String,
}

/// The session key the flash region reads and the notice route writes.
pub const FLASH_KEY: &str = "suprnova-ui.flash";

#[suprnova::view(path = "live/feedback.html")]
struct FeedbackView<'a> {
    bootstrap: &'a TrustedHtml,
    gallery: &'a TrustedHtml,
    messages: Vec<FlashMessage>,
}

#[suprnova::view(path = "live/navigation.html")]
struct NavigationView<'a> {
    bootstrap: &'a TrustedHtml,
    gallery: &'a TrustedHtml,
}

/// The feedback gallery: the suprnova-ui base is opted in, every feedback
/// component is mounted once, the empty state's reason comes from the
/// `reason` query (FDB-003) and the flash region shows what the previous
/// request left in the session (FDB-004).
pub async fn feedback(request: Request, mounts: &FeedbackMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let reason = request
            .query_param("reason")
            .unwrap_or_else(|| "empty".to_owned());
        let messages = session_mut(|session| session.get_flash::<Vec<FlashMessage>>(FLASH_KEY))
            .flatten()
            .unwrap_or_default();
        let mut document = LiveDocument::from_request(&request)?;
        let parameters = CanonicalValue::Object(BTreeMap::from([(
            "reason".to_owned(),
            CanonicalValue::String(reason),
        )]));
        let gallery = document
            .mount(&mounts.gallery, parameters, MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        document
            .render(
                view("live/feedback.html")?,
                &FeedbackView {
                    bootstrap: bootstrap.html(),
                    gallery: gallery.html(),
                    messages,
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}

/// `GET /live/feedback/notice`: leaves one success message in the session
/// flash and redirects to the gallery, the way an accepted form post does;
/// the gallery renders it once and a reload finds nothing.
pub async fn feedback_notice(_request: Request) -> Response {
    session_mut(|session| {
        session.flash(
            FLASH_KEY,
            vec![FlashMessage {
                variant: "success".to_owned(),
                text: "Your changes were saved".to_owned(),
            }],
        );
    });
    Ok(HttpResponse::new()
        .status(303)
        .header("Location", FEEDBACK_PATH))
}

/// The navigation gallery: every navigation component mounted once; the
/// page number comes from the `page` query so a reflected URL reloads onto
/// the same page (NAV-003).
pub async fn navigation(request: Request, mounts: &NavigationMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let page = request
            .query_param("page")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        let mut document = LiveDocument::from_request(&request)?;
        let parameters = CanonicalValue::Object(BTreeMap::from([(
            "page".to_owned(),
            CanonicalValue::number(f64::from(page))
                .map_err(|_| FrameworkError::internal("Live navigation page number"))?,
        )]));
        let gallery = document
            .mount(&mounts.gallery, parameters, MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        document
            .render(
                view("live/navigation.html")?,
                &NavigationView {
                    bootstrap: bootstrap.html(),
                    gallery: gallery.html(),
                },
                intent()?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(failed)
}

/// The data-display gallery page: the presentational island and the datatable island.
#[suprnova::view(path = "live/data-display.html")]
struct DataDisplayView<'a> {
    bootstrap: &'a TrustedHtml,
    gallery: &'a TrustedHtml,
    table: &'a TrustedHtml,
}

/// `GET /live/data-display`: every data-display component, with the
/// datatable mounted from the query so a shared URL renders the same view.
pub async fn data_display(request: Request, mounts: &DataDisplayMounts) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let text = |name: &str| request.query_param(name).unwrap_or_default();
        let page = request
            .query_param("page")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1);
        let mut document = LiveDocument::from_request(&request)?;
        let gallery = document
            .mount(&mounts.gallery, parameters(), MountFlags::empty())
            .await?;
        let parameters = CanonicalValue::Object(BTreeMap::from([
            ("sort".to_owned(), CanonicalValue::String(text("sort"))),
            ("direction".to_owned(), CanonicalValue::String(text("dir"))),
            ("filter".to_owned(), CanonicalValue::String(text("filter"))),
            (
                "page".to_owned(),
                CanonicalValue::number(f64::from(page))
                    .map_err(|_| FrameworkError::internal("Live datatable page number"))?,
            ),
        ]));
        let table = document
            .mount(&mounts.table, parameters, MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
        document
            .render(
                view("live/data-display.html")?,
                &DataDisplayView {
                    bootstrap: bootstrap.html(),
                    gallery: gallery.html(),
                    table: table.html(),
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
