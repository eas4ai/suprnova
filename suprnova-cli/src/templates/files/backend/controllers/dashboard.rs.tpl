//! Dashboard controller.
//!
//! Renders the page a sign-in lands on. The signed-in user reaches it, as
//! every page, through the shared `auth` prop (`crate::props::shared`). Its
//! own two props each travel the way their cost and their place on the
//! page call for:
//!
//! - `stats`, two counts over the user's notes, is deferred: the first
//!   response leaves it out and names it under `deferredProps`, and the
//!   client asks for it as soon as the page shows, with the page's
//!   `Deferred` fallback in its place until then. The page also polls it
//!   (`usePoll` with `only: ['stats']`), and each poll runs only this
//!   prop's queries.
//! - `recent_notes`, the user's five newest notes, is optional: no
//!   response carries it until the client asks for it by name, which the
//!   page's `WhenVisible` block does once it scrolls into view.
//!
//! Both start from `Note::owned_by`, so the numbers and the list are the
//! signed-in user's own.

use chrono::Utc;
use suprnova::{Auth, FrameworkError, InertiaProps, InertiaResponse, Request, Response, handler};

use super::notes::NoteSummary;
use crate::models::note::Note;
use crate::models::user::User;

/// How many notes `recent_notes` lists.
const RECENT_NOTES: u64 = 5;

/// The `Dashboard` page's props, for `suprnova generate-types`. The handler
/// sends neither with the page: `stats` arrives with the client's deferred
/// request, and `recent_notes` when the page asks for it.
#[derive(InertiaProps)]
pub struct DashboardProps {
    pub stats: Stats,
    pub recent_notes: Vec<NoteSummary>,
}

/// Counts over the signed-in user's own notes: every note, and the notes
/// written today (the UTC day the server is in).
#[derive(InertiaProps)]
pub struct Stats {
    pub notes: i64,
    pub written_today: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    // The registered user provider resolves the typed `User` from the
    // session id; `user_as` downcasts the `Authenticatable` for us.
    let user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    // The resolvers below run after this handler returns, when the
    // response is resolved, so each takes the id rather than the request.
    let user_id = user.id;

    Ok(InertiaResponse::new("Dashboard")
        .defer("stats", move || note_stats(user_id))
        .optional("recent_notes", move || recent_notes(user_id))
        .resolve(&req)
        .await?)
}

/// `stats`: how many notes the user has, and how many they wrote today.
async fn note_stats(user_id: u64) -> Result<Stats, FrameworkError> {
    let notes = Note::owned_by(user_id).count().await?;
    let written_today = Note::owned_by(user_id)
        .where_date("created_at", Utc::now().date_naive())
        .count()
        .await?;
    Ok(Stats {
        notes,
        written_today,
    })
}

/// `recent_notes`: the user's newest notes, newest first. The id orders
/// them, since two notes written in the same second share a `created_at`.
async fn recent_notes(user_id: u64) -> Result<Vec<NoteSummary>, FrameworkError> {
    let notes = Note::owned_by(user_id)
        .latest_by("id")
        .limit(RECENT_NOTES)
        .get()
        .await?;
    Ok(notes
        .into_vec()
        .into_iter()
        .map(NoteSummary::from)
        .collect())
}
