//! Notes: the signed-in user's own notes, listed, shown and written.
//!
//! Every query here starts from `Note::owned_by(user.id)`. A handler
//! therefore never reads a note another user wrote, and asking for one by
//! id is the same `404` as asking for an id that does not exist.
//!
//! The list pages by cursor (`cursor_paginate`), which the page's
//! `InfiniteScroll` reads from the scroll metadata `Inertia::paginate`
//! attaches: each next page is the rows after the last one shown, so a
//! note written while the user scrolls neither repeats nor skips a row.

use serde::Deserialize;
use suprnova::{
    Auth, CursorPaginator, FormRequest, FrameworkError, Inertia, InertiaProps, Model, Request,
    Response, Validate, ValidationErrors, attrs, handler, inertia_response,
};

use crate::models::note::Note;
use crate::models::user::User;
use crate::props::flash::Toast;

/// How many notes each page of the list holds.
const NOTES_PER_PAGE: u64 = 10;

/// A note as a list shows it: the notes list's rows and the dashboard's
/// recent notes. A row's link renders `Notes/Show` from it before the
/// server answers, and the body arrives with that answer.
#[derive(InertiaProps)]
pub struct NoteSummary {
    pub id: u64,
    pub title: String,
    pub created_at: String,
}

impl From<Note> for NoteSummary {
    fn from(note: Note) -> Self {
        Self {
            id: note.id,
            title: note.title,
            created_at: note.created_at.map(|date| date.to_rfc3339()).unwrap_or_default(),
        }
    }
}

/// A whole note, as `Notes/Show` shows it.
#[derive(InertiaProps)]
pub struct NoteView {
    pub id: u64,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
}

impl From<Note> for NoteView {
    fn from(note: Note) -> Self {
        Self {
            id: note.id,
            title: note.title,
            body: note.body,
            created_at: note.created_at.map(|date| date.to_rfc3339()).unwrap_or_default(),
        }
    }
}

/// The `Notes/Index` page's props, for `suprnova generate-types`. The
/// handler sends them through `Inertia::paginate`, which puts each page's
/// rows under `notes` and their cursors in the scroll metadata.
#[derive(InertiaProps)]
pub struct NotesIndexProps {
    pub notes: Vec<NoteSummary>,
    pub search: String,
}

/// The `Notes/Show` page's props.
#[derive(InertiaProps)]
pub struct NotesShowProps {
    pub note: NoteView,
}

/// The note form's fields. A missing field reads as empty, so it fails
/// the rules below rather than the parse.
#[derive(Deserialize, Validate)]
pub struct StoreNoteRequest {
    #[serde(default)]
    #[validate(length(
        min = 1,
        max = 255,
        message = "Give the note a title of 1 to 255 characters."
    ))]
    pub title: String,
    #[serde(default)]
    #[validate(length(max = 10000, message = "Keep the note to 10000 characters."))]
    pub body: Option<String>,
}

impl FormRequest for StoreNoteRequest {
    /// A title of spaces alone is no title. Laravel trims input before
    /// `required` sees it; this is the same check.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.title.trim().is_empty() {
            let mut errs = ValidationErrors::new();
            errs.add("title", "Give the note a title of 1 to 255 characters.");
            return Err(errs);
        }
        Ok(())
    }
}

/// The signed-in user. The routes here run behind the `auth` middleware,
/// so `None` means the session lost its user between the guard and the
/// handler; refusing is the safe answer.
async fn current_user() -> Result<User, FrameworkError> {
    Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)
}

/// The `LIKE` pattern for text that contains `search` in any case, or
/// `None` when `search` is empty and every note matches.
///
/// `!` escapes the wildcards, so `50%` finds the text `50%` rather than
/// every note that starts with `50`. It is `!` rather than `\` because
/// MySQL reads a backslash inside a quoted string as an escape of its own.
/// SQLite's `LOWER` folds ASCII letters only, so on SQLite a search
/// ignores the case of ASCII letters alone.
fn contains_pattern(search: &str) -> Option<String> {
    if search.is_empty() {
        return None;
    }
    let mut pattern = String::from("%");
    for c in search.to_lowercase().chars() {
        if matches!(c, '!' | '%' | '_') {
            pattern.push('!');
        }
        pattern.push(c);
    }
    pattern.push('%');
    Some(pattern)
}

/// `GET /notes?search=...` - the user's notes whose title or body contains
/// `search`, a page at a time, with `search` sent back for the input.
#[handler]
pub async fn index(req: Request) -> Response {
    let user = current_user().await?;
    let search = req.query_param("search").unwrap_or_default();

    let mut notes = Note::owned_by(user.id);
    if let Some(pattern) = contains_pattern(&search) {
        // The pattern travels as a bound value; only the column names are
        // in the SQL text. `?` is the portable placeholder.
        notes = notes.where_raw(
            "(LOWER(title) LIKE ? ESCAPE '!' OR LOWER(body) LIKE ? ESCAPE '!')",
            vec![pattern.clone().into(), pattern.into()],
        );
    }
    let page = notes.cursor_paginate(NOTES_PER_PAGE).await?;

    // Send each row as a `NoteSummary`. The cursors stay valid: they hold a
    // row's id, which the projection keeps. The scroll metadata reads the
    // current cursor from the request's `cursor` parameter, the one
    // `cursor_paginate` reads.
    let rows = CursorPaginator::new(
        page.data.into_iter().map(NoteSummary::from).collect(),
        page.per_page,
        page.next_cursor,
        page.prev_cursor,
    );

    Ok(Inertia::paginate("Notes/Index", "notes", rows)
        .with("search", search)
        .resolve(&req)
        .await?)
}

/// `GET /notes/{id}` - one of the user's notes, or the framework's `404`.
#[handler]
pub async fn show(req: Request, id: u64) -> Response {
    let user = current_user().await?;
    let note = Note::owned_by(user.id)
        .filter("id", id)
        .first_or_fail()
        .await?;
    inertia_response!(&req, "Notes/Show", NotesShowProps { note: note.into() })
}

/// `POST /notes` - write a note for the signed-in user and return to the
/// form's page, keeping its search and showing the saved toast.
///
/// A failed rule answers an Inertia visit with a `303` back to the form,
/// the errors flashed for the page's `Form` component.
#[handler]
pub async fn store(form: StoreNoteRequest) -> Response {
    let user = current_user().await?;
    // An empty body is no body, as Laravel's `ConvertEmptyStringsToNull`
    // makes it.
    let body = form.body.filter(|body| !body.trim().is_empty());
    <Note as Model>::create(attrs! {
        user_id: user.id,
        title: form.title.trim(),
        body: body,
    })
    .await?;

    Inertia::flash("toast", Toast::success("Note saved."))?;
    Inertia::back(302, Some("/notes")).into()
}
