//! Notes: the signed-in user's own notes, listed, shown and written.
//!
//! Ordinary reads use `Note::owned_by(user.id)` and search uses the
//! index's viewer scope plus the owner filter. A handler never reads a note
//! another user wrote, and asking for one by
//! id is the same `404` as asking for an id that does not exist.
//!
//! The ordinary list pages with `cursor_paginate`; a searched list uses
//! an encrypted query and ranked offset. `InfiniteScroll` reads either
//! cursor from the scroll metadata `Inertia::paginate` attaches. Ranking
//! remains stable across pages while the index is unchanged.

use serde::Deserialize;
use suprnova::serde_json;
use suprnova::{
    Auth, CursorPaginator, FormRequest, FrameworkError, Inertia, InertiaProps, Model, Request,
    Response, Searchable, Validate, ValidationErrors, attrs, handler, inertia_response,
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
    /// Present for ranked search rows so frontends can render escaped matching text.
    pub highlight: Option<NoteHighlight>,
}

/// Escaped title and body with the search engine's validated highlight tags.
#[derive(InertiaProps)]
pub struct NoteHighlight {
    /// The core escapes source text before inserting its validated matching tags.
    pub title: String,
    /// The body preview follows the same escaped formatting contract as the title.
    pub body: String,
}

impl From<Note> for NoteSummary {
    fn from(note: Note) -> Self {
        Self {
            highlight: None,
            id: note.id,
            title: note.title,
            created_at: note
                .created_at
                .map(|date| date.to_rfc3339())
                .unwrap_or_default(),
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
            created_at: note
                .created_at
                .map(|date| date.to_rfc3339())
                .unwrap_or_default(),
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

/// `GET /notes?search=...` - the user's notes ranked by title and body for
/// `search`, a page at a time, with `search` sent back for the input.
#[handler]
pub async fn index(req: Request) -> Response {
    let user = current_user().await?;
    let search = req.query_param("search").unwrap_or_default();

    let rows = if search.is_empty() {
        let page = Note::owned_by(user.id)
            .cursor_paginate(NOTES_PER_PAGE)
            .await?;
        CursorPaginator::new(
            page.data.into_iter().map(NoteSummary::from).collect(),
            page.per_page,
            page.next_cursor,
            page.prev_cursor,
        )
    } else {
        // Search ranks one list. Its encrypted cursor carries the query and
        // offset so a cursor from another search cannot silently skip hits.
        let offset = match req.query_param("cursor") {
            Some(cursor) => {
                let payload =
                    CursorPaginator::<NoteSummary>::decode_cursor(&cursor).map_err(|_| {
                        FrameworkError::param("cursor must be a valid encrypted search cursor")
                    })?;
                let (query, offset): (String, usize) = serde_json::from_str(&payload)
                    .map_err(|_| FrameworkError::param("cursor must contain a search offset"))?;
                if query != search {
                    return Err(
                        FrameworkError::param("cursor belongs to another search query").into(),
                    );
                }
                offset
            }
            None => 0,
        };
        let per_page = NOTES_PER_PAGE as usize;
        let result = Note::search(&search)
            .filter(format!("user_id = {}", user.id))
            .highlight(["title", "body"])
            .offset(offset)
            .limit(per_page)
            .get()
            .await?;
        // Lexical search counts all matches, so impressions cover exactly this page.
        let more = result.total.value() > (offset as u64).saturating_add(result.hits.len() as u64);
        let notes = result
            .hits
            .into_iter()
            .map(|hit| {
                let mut note = NoteSummary::from(hit.model);
                note.highlight = Some(NoteHighlight {
                    title: hit.formatted.get("title").cloned().unwrap_or_default(),
                    body: hit.formatted.get("body").cloned().unwrap_or_default(),
                });
                note
            })
            .collect();
        let cursor = |offset| -> Result<String, FrameworkError> {
            let payload = serde_json::to_string(&(&search, offset)).map_err(|error| {
                FrameworkError::internal(format!("encoding search cursor: {error}"))
            })?;
            CursorPaginator::<NoteSummary>::try_encode_cursor(&payload)
        };
        CursorPaginator::new(
            notes,
            NOTES_PER_PAGE,
            more.then(|| cursor(offset.saturating_add(per_page)))
                .transpose()?,
            (offset > 0)
                .then(|| cursor(offset.saturating_sub(per_page)))
                .transpose()?,
        )
    };

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
