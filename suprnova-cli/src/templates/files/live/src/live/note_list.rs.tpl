//! `NoteList`: the signed-in user's notes on `/notes`, ranked when searched.
//!
//! The search box filters by title and text. Its applied value is bound to
//! the URL's `?search=` query: the document mounts the list from it, and
//! every search reflects the new value into the address bar without a
//! history entry, so a shared or reloaded URL shows the same list. "Load
//! more" adds the next page to the list. The rows are read on every render
//! and never enter the snapshot, so the list a render shows is always the
//! user's notes as the database holds them.
//!
//! Ordinary reads use `Note::owned_by(user.id)`. Searches use the index's
//! viewer scope and the owner filter, so the list never holds another
//! user's note.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use suprnova::Searchable;
use suprnova::live::action::{OutcomeMetadata, action_result, url_intent};
use suprnova::live::{
    ActionOutcome, ActionResult, CanonicalValue, ComponentError, LiveComponent, live,
};

use super::{current_user, failed};
use crate::models::note::Note;

/// The checked loop-key filter the rows use.
pub mod filters {
    pub use suprnova::view::filters::{live_key, trusted_html};
}

/// How many notes each page of the list adds.
const NOTES_PER_PAGE: u64 = 10;

/// A note as a list shows it: the notes list's rows and the dashboard's
/// recent notes.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NoteRow {
    /// The note's id, for its link.
    pub id: u64,
    /// The row's stable key across morphs.
    pub key: String,
    /// The note's title.
    pub title: String,
    /// Escaped and highlighted title from the framework search formatter.
    pub highlight_title: Option<String>,
    /// Escaped and highlighted body from the framework search formatter.
    pub highlight_body: Option<String>,
    /// When the note was written, as RFC 3339 for the `datetime` attribute.
    pub created_at: String,
    /// When the note was written, as the row prints it.
    pub created_on: String,
}

impl From<Note> for NoteRow {
    fn from(note: Note) -> Self {
        Self {
            key: format!("note-{}", note.id),
            id: note.id,
            title: note.title,
            highlight_title: None,
            highlight_body: None,
            created_at: note
                .created_at
                .map(|date| date.to_rfc3339())
                .unwrap_or_default(),
            created_on: note
                .created_at
                .map(|date| date.format("%b %-d, %Y").to_string())
                .unwrap_or_default(),
        }
    }
}

/// The searchable list, rendered by `live/note_list.html`.
#[derive(LiveComponent)]
#[live(
    name = "{package_name}.note-list",
    view = "live/note_list.html",
    minimum_protocol_version = 2
)]
pub struct NoteList {
    /// What the search box holds, proposed as the user types.
    #[model(debounce = 250)]
    query: String,
    /// The search the list applies, bound to the URL's `?search=`.
    #[url(key = "search")]
    search: String,
    /// How many pages of notes the list shows.
    pages: u64,
    /// The notes this render shows, read in [`NoteList::load`].
    #[server_only]
    notes: Vec<NoteRow>,
    /// Whether more notes match than the list shows.
    #[server_only]
    more: bool,
}

#[live]
impl NoteList {
    /// Mounts the list from the document's `?search=` query.
    #[mount]
    pub fn mount(search: String) -> Self {
        let search = search.trim().to_owned();
        Self {
            query: search.clone(),
            search,
            pages: 1,
            notes: Vec::new(),
            more: false,
        }
    }

    /// Applies what the search box holds and starts the list again.
    #[action]
    pub fn search(&mut self) -> Result<ActionResult, ComponentError> {
        self.search = self.query.trim().to_owned();
        self.pages = 1;
        self.reflect()
    }

    /// Empties the search box and shows every note again.
    #[action]
    pub fn clear_search(&mut self) -> Result<ActionResult, ComponentError> {
        self.query.clear();
        self.search.clear();
        self.pages = 1;
        self.reflect()
    }

    /// Adds the next page of notes to the list.
    #[action]
    pub fn load_more(&mut self) {
        self.pages = self.pages.max(1).saturating_add(1);
    }

    /// Reads the requested pages; search totals keep impressions limited to shown notes.
    #[rendering]
    pub async fn load(&mut self) -> Result<(), ComponentError> {
        let user = current_user().await?;
        let shown = self.pages.max(1).saturating_mul(NOTES_PER_PAGE);
        let limit = usize::try_from(shown).unwrap_or(usize::MAX);
        let (mut rows, more): (Vec<NoteRow>, bool) = if self.search.is_empty() {
            let rows: Vec<NoteRow> = Note::owned_by(user.id)
                .latest_by("id")
                .limit(shown.saturating_add(1))
                .get()
                .await
                .map_err(|error| failed("listing notes", error))?
                .into_vec()
                .into_iter()
                .map(NoteRow::from)
                .collect();
            let more = rows.len() > limit;
            (rows, more)
        } else {
            let result = Note::search(&self.search)
                .filter(format!("user_id = {}", user.id))
                .highlight(["title", "body"])
                .limit(limit)
                .get()
                .await
                .map_err(|error| failed("searching notes", error))?;
            let more = result.total.value() > result.hits.len() as u64;
            let rows = result
                .hits
                .into_iter()
                .map(|hit| {
                    let mut row = NoteRow::from(hit.model);
                    row.highlight_title = hit.formatted.get("title").cloned();
                    row.highlight_body = hit.formatted.get("body").cloned();
                    row
                })
                .collect();
            (rows, more)
        };
        let shown = usize::try_from(shown).unwrap_or(usize::MAX);
        self.more = more;
        rows.truncate(shown);
        self.notes = rows;
        Ok(())
    }

    /// Trusts only escaped fields produced by the search formatter, never raw note text.
    pub fn highlight(
        &self,
        html: &str,
    ) -> Result<suprnova::view::TrustedHtml, suprnova::FrameworkError> {
        let reason = suprnova::view::TrustedMarkupReason::new(
            "escaped search fields with validated highlight tags",
        )
        .map_err(|error| {
            suprnova::FrameworkError::internal(format!("search highlight reason: {error}"))
        })?;
        suprnova::view::TrustedHtml::framework_generated(html.to_owned(), reason).map_err(|error| {
            suprnova::FrameworkError::internal(format!("search highlight markup: {error}"))
        })
    }

    /// The text the list shows when the search matched nothing.
    pub fn no_results(&self) -> String {
        format!("No note's title or text matches \"{}\".", self.search)
    }

    /// A render whose result puts the applied search into the URL, or takes
    /// it out when the search is empty.
    fn reflect(&self) -> Result<ActionResult, ComponentError> {
        let mut query = BTreeMap::new();
        if !self.search.is_empty() {
            query.insert(
                "search".to_owned(),
                CanonicalValue::String(self.search.clone()),
            );
        }
        let url = url_intent(CanonicalValue::Object(query))
            .map_err(|error| failed("reflecting the search into the URL", error))?;
        let metadata = OutcomeMetadata::new(Vec::new(), Vec::new(), Vec::new(), Some(url))
            .map_err(|error| failed("reflecting the search into the URL", error))?;
        action_result::<Self>(ActionOutcome::Render, metadata)
            .map_err(|error| failed("reflecting the search into the URL", error))
    }
}
