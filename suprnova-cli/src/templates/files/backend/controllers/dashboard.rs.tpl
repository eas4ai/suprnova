//! Dashboard controller.
//!
//! Renders the page a sign-in lands on. Its three props each travel the
//! way their cost and their place on the page call for:
//!
//! - `user`, the signed-in user, goes with the page.
//! - `stats`, two counts over the user's notes, is deferred: the first
//!   response leaves it out and names it under `deferredProps`, and the
//!   client asks for it as soon as the page shows, with the page's
//!   `Deferred` fallback in its place until then. The page also polls it
//!   (`usePoll` with `only: ['stats']`), and each poll runs only this
//!   prop's query.
//! - `recent_notes`, the user's five newest notes, is optional: no
//!   response carries it until the client asks for it by name, which the
//!   page's `WhenVisible` block does once it scrolls into view.
//!
//! Both queries start from `Note::owned_by`, so the numbers and the list
//! are the signed-in user's own.

use chrono::{DateTime, Utc};
use suprnova::{Auth, FrameworkError, InertiaProps, InertiaResponse, Request, Response, handler};

use crate::models::note::Note;
use crate::models::user::User;

/// How many notes `recent_notes` lists.
const RECENT_NOTES: u64 = 5;

/// The signed-in user, as the dashboard and the name form show them.
#[derive(InertiaProps)]
pub struct UserInfo {
    pub id: u64,
    pub name: String,
    pub email: String,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            name: user.name,
            email: user.email,
        }
    }
}

/// Counts over the signed-in user's own notes: every note, and the notes
/// written today (the UTC day the server is in).
#[derive(InertiaProps)]
pub struct NoteStats {
    pub notes: i64,
    pub written_today: i64,
}

/// One of the signed-in user's newest notes, as the dashboard lists it.
#[derive(InertiaProps)]
pub struct RecentNote {
    pub id: u64,
    pub title: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<Note> for RecentNote {
    fn from(note: Note) -> Self {
        Self {
            id: note.id,
            title: note.title,
            created_at: note.created_at,
        }
    }
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
        .with("user", UserInfo::from(user))
        .defer("stats", move || note_stats(user_id))
        .optional("recent_notes", move || recent_notes(user_id))
        .resolve(&req)
        .await?)
}

/// `stats`: how many notes the user has, and how many they wrote today.
async fn note_stats(user_id: u64) -> Result<NoteStats, FrameworkError> {
    let notes = Note::owned_by(user_id).count().await?;
    let written_today = Note::owned_by(user_id)
        .where_date("created_at", Utc::now().date_naive())
        .count()
        .await?;
    Ok(NoteStats {
        notes,
        written_today,
    })
}

/// `recent_notes`: the user's newest notes, newest first. The id orders
/// them, since two notes written in the same second share a `created_at`.
async fn recent_notes(user_id: u64) -> Result<Vec<RecentNote>, FrameworkError> {
    let notes = Note::owned_by(user_id)
        .latest_by("id")
        .limit(RECENT_NOTES)
        .get()
        .await?;
    Ok(notes.into_vec().into_iter().map(RecentNote::from).collect())
}
