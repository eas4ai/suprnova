//! The dashboard: the page a sign-in lands on.
//!
//! It mounts two identity-bound islands: the display-name form
//! (`live::profile_name`) and the note counts with the five newest notes
//! (`live::note_stats`), which refresh without a page load whenever the
//! user saves a note.

use suprnova::live::{LiveDocument, MountFlags};
use suprnova::{FrameworkError, HttpResponse, Request, Response, handler};

use super::auth::no_parameters;
use crate::live::mounts;
use crate::views::{self, DashboardPage, Frame, Nav};

/// `GET /dashboard`.
#[handler]
pub async fn index(req: Request) -> Response {
    Ok(page(&req).await?)
}

async fn page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mounts = mounts()?;
    let mut document = LiveDocument::from_request(req)?;
    let profile = document
        .mount(&mounts.profile, no_parameters(), MountFlags::empty())
        .await?;
    let stats = document
        .mount(&mounts.stats, no_parameters(), MountFlags::empty())
        .await?;
    let frame = Frame::live(&mut document, "Dashboard", Nav::Dashboard).await?;
    let page = DashboardPage {
        frame,
        profile: profile.html().clone(),
        stats: stats.html().clone(),
    };
    views::render(document, "live/pages/dashboard.html", &page)
}
