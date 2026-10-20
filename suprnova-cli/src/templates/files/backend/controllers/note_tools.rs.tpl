//! The notes tools: `search_notes` reads the user's own notes and
//! `create_note` writes one, for an MCP client the user approved, an agent
//! the user claimed, or an agent in the user's browser on the notes page,
//! which offers both (`routes.rs`).
//!
//! Each call acts for one user: the one signed in, or the one a claimed
//! agent is bound to. Every query starts from that user's notes, so no
//! caller reads or writes another user's. A token needs the tool's scope
//! from `crate::agents::scope`, and an agent no user has claimed holds none.
//!
//! Note text is user content, so `search_notes` is marked
//! `untrusted_content`: an agent reads what it answers as data, never as
//! instructions.

use suprnova::serde_json;
use suprnova::{
    FormRequest, FrameworkError, HttpResponse, Model, Response, Searchable, ToolCaller,
    ValidationErrors, attrs, handler, tool, tool_input, tool_output,
};

use crate::agents::scope;
use crate::models::note::Note;

/// How many notes `search_notes` answers when the call names no limit.
const DEFAULT_LIMIT: u32 = 10;

/// What `search_notes` takes. The notes page sends it in the query of a
/// `GET`, and an MCP client as the call's arguments; the form request reads
/// either.
#[tool_input]
pub struct SearchNotesInput {
    /// Words to find in the titles and text of the notes. Leave it empty to
    /// list the newest notes.
    #[serde(default)]
    #[validate(length(max = 200, message = "Keep the search to 200 characters."))]
    pub query: String,
    /// How many notes to answer, 1 to 50. Ten when left out.
    #[serde(default)]
    #[validate(range(min = 1, max = 50, message = "Ask for 1 to 50 notes."))]
    pub limit: Option<u32>,
}

/// What `create_note` takes: the notes form's two fields, with its rules.
#[tool_input]
#[form_request(custom_hooks)]
pub struct CreateNoteInput {
    /// The note's title.
    #[serde(default)]
    #[validate(length(
        min = 1,
        max = 255,
        message = "Give the note a title of 1 to 255 characters."
    ))]
    pub title: String,
    /// The note's text, which may be left out.
    #[serde(default)]
    #[validate(length(max = 10000, message = "Keep the note to 10000 characters."))]
    pub body: Option<String>,
}

impl FormRequest for CreateNoteInput {
    /// A title of spaces alone is no title, as the notes form decides.
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.title.trim().is_empty() {
            let mut errs = ValidationErrors::new();
            errs.add("title", "Give the note a title of 1 to 255 characters.");
            return Err(errs);
        }
        Ok(())
    }
}

/// A note as the tools answer it.
#[tool_output]
pub struct NoteItem {
    /// The note's id, which its page `/notes/{id}` names.
    pub id: u64,
    /// The note's title.
    pub title: String,
    /// The note's text, if it has any.
    pub body: Option<String>,
    /// When the note was written, as RFC 3339.
    pub created_at: String,
}

impl From<Note> for NoteItem {
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

/// What `search_notes` answers. A list rides in an object, so clients of
/// every MCP revision read it as structured content.
#[tool_output]
pub struct NoteSearchResults {
    /// The notes found, best match first, or newest first without words.
    pub notes: Vec<NoteItem>,
}

/// The id of the user `caller` acts for. Both tools need a scope that no
/// agent bound to nobody holds, so such a caller does not reach a handler;
/// refusing one is the safe answer if it ever did.
fn acting_user(caller: &ToolCaller) -> Result<u64, FrameworkError> {
    caller
        .user_id()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| {
            FrameworkError::domain(
                "These tools act for a user, and this caller acts for none.",
                403,
            )
        })
}

/// `value` as the tool's JSON answer.
fn answer(value: serde_json::Result<serde_json::Value>) -> Response {
    let body =
        value.map_err(|error| FrameworkError::internal(format!("encoding the notes: {error}")))?;
    Ok(HttpResponse::json(body))
}

/// `GET /notes/search?query=...&limit=...` - the tool `search_notes`.
#[tool(
    name = "search_notes",
    title = "Search notes",
    description = "Search the user's notes by the words in their titles and text, best match first. Leave the query empty to list the newest notes.",
    callers = [user, agent],
    scopes = [scope::NOTES_READ],
    input = SearchNotesInput,
    output = NoteSearchResults,
    annotations(read_only, untrusted_content),
)]
#[handler]
pub async fn search(caller: ToolCaller, form: SearchNotesInput) -> Response {
    let user_id = acting_user(&caller)?;
    let limit = form.limit.unwrap_or(DEFAULT_LIMIT);
    let query = form.query.trim();
    let notes: Vec<NoteItem> = if query.is_empty() {
        Note::owned_by(user_id)
            .latest_by("id")
            .limit(u64::from(limit))
            .get()
            .await?
            .into_vec()
            .into_iter()
            .map(NoteItem::from)
            .collect()
    } else {
        // The notes page's own search: the index's viewer scope, and the
        // owner filter as well.
        Note::search(query)
            .filter(format!("user_id = {user_id}"))
            .limit(usize::try_from(limit).unwrap_or(usize::MAX))
            .get()
            .await?
            .hits
            .into_iter()
            .map(|hit| NoteItem::from(hit.model))
            .collect()
    };
    answer(serde_json::to_value(NoteSearchResults { notes }))
}

/// `POST /notes/tools` - the tool `create_note`, answering in JSON. The
/// notes page's own form posts to `POST /notes`, which redirects back.
#[tool(
    name = "create_note",
    title = "Write a note",
    description = "Write a note for the user, with a title and, if you like, some text. Answers the note written.",
    callers = [user, agent],
    scopes = [scope::NOTES_WRITE],
    input = CreateNoteInput,
    output = NoteItem,
    annotations(read_only = false, destructive = false),
)]
#[handler]
pub async fn create(caller: ToolCaller, form: CreateNoteInput) -> Response {
    let user_id = acting_user(&caller)?;
    // The notes form's write, through the same model, so the search index
    // takes the note. Empty text is no text, as Laravel's
    // `ConvertEmptyStringsToNull` makes it.
    let body = form.body.filter(|body| !body.trim().is_empty());
    let note = <Note as Model>::create(attrs! {
        user_id: user_id,
        title: form.title.trim(),
        body: body,
    })
    .await?;
    answer(serde_json::to_value(NoteItem::from(note)))
}
