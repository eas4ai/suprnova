//! Notes: the signed-in user's own notes, listed, shown and written.
//!
//! `/notes` mounts the note form (`live::note_form`) and the searchable
//! list (`live::note_list`). `/notes/{id}` shows one note. Every query
//! starts from `Note::owned_by(user.id)`, so asking for another user's note
//! by id is the same `404` as asking for an id that does not exist.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveDocument, MountFlags};
use suprnova::{Auth, FrameworkError, HttpResponse, Request, Response, handler};

use super::auth::no_parameters;
use crate::live::mounts;
use crate::models::note::Note;
use crate::models::user::User;
use crate::views::{self, Frame, Nav, NoteDetail, NoteShowPage, NotesIndexPage};

/// `GET /notes?search=...`: the form and the list, filtered by `search`.
#[handler]
pub async fn index(req: Request) -> Response {
    Ok(index_page(&req).await?)
}

async fn index_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mounts = mounts()?;
    let search = req.query_param("search").unwrap_or_default();
    let list_parameters = CanonicalValue::Object(BTreeMap::from([(
        "search".to_owned(),
        CanonicalValue::String(search),
    )]));
    let mut document = LiveDocument::from_request(req)?;
    let form = document
        .mount(&mounts.note_form, no_parameters(), MountFlags::empty())
        .await?;
    let list = document
        .mount(&mounts.note_list, list_parameters, MountFlags::empty())
        .await?;
    let frame = Frame::live(&mut document, "Notes", Nav::Notes).await?;
    let page = NotesIndexPage {
        frame,
        form: form.html().clone(),
        list: list.html().clone(),
    };
    views::render(document, "live/pages/notes-index.html", &page)
}

/// `GET /notes/{id}`: one of the user's notes, or the framework's `404`.
#[handler]
pub async fn show(req: Request, id: u64) -> Response {
    Ok(show_page(&req, id).await?)
}

async fn show_page(req: &Request, id: u64) -> Result<HttpResponse, FrameworkError> {
    // Behind the `auth` middleware, so `None` means the session lost its
    // user between the guard and here; refusing is the safe answer.
    let user = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let note = Note::owned_by(user.id)
        .filter("id", id)
        .first_or_fail()
        .await?;
    let note = NoteDetail {
        title: note.title,
        body: note.body,
        created_at: note
            .created_at
            .map(|date| date.to_rfc3339())
            .unwrap_or_default(),
        created_on: note
            .created_at
            .map(|date| date.format("%b %-d, %Y").to_string())
            .unwrap_or_default(),
    };
    let mut document = LiveDocument::from_request(req)?;
    let frame = Frame::live(&mut document, &note.title, Nav::Notes).await?;
    views::render(
        document,
        "live/pages/notes-show.html",
        &NoteShowPage { frame, note },
    )
}
