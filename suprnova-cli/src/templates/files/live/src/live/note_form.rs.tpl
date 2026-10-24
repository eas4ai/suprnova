//! `NoteForm`: the form on `/notes` that writes a note.
//!
//! The action checks the title and the text, writes the note for the
//! signed-in user, tells the user's open dashboards to refresh, and reloads
//! the list with a toast. A failed rule stays on the form with its message.

use suprnova::live::{ActionResult, ComponentError, LiveComponent, live};
use suprnova::{Model, Validate, attrs};

use super::{FieldErrors, current_user, failed, note_stats, redirect};
use crate::flash::Toast;
use crate::models::note::Note;

/// The rules a note must pass. The title is checked once trimmed, so a
/// title of spaces alone is no title, as Laravel's trimming makes it.
#[derive(Validate)]
struct NoteInput {
    #[validate(length(
        min = 1,
        max = 255,
        message = "Give the note a title of 1 to 255 characters."
    ))]
    title: String,
    #[validate(length(max = 10000, message = "Keep the note to 10000 characters."))]
    body: String,
}

/// The note form, rendered by `live/note_form.html`.
#[derive(LiveComponent)]
#[live(name = "{package_name}.note-form", view = "live/note_form.html")]
pub struct NoteForm {
    /// The note's title.
    #[model]
    title: String,
    /// The note's text, which may be empty.
    #[model]
    body: String,
    /// The message under each control the last save failed.
    errors: FieldErrors,
}

#[live]
impl NoteForm {
    /// Writes the note, or shows why it was not written.
    #[action]
    pub async fn save(&mut self) -> Result<ActionResult, ComponentError> {
        let input = NoteInput {
            title: self.title.trim().to_owned(),
            body: self.body.clone(),
        };
        if let Err(errors) = input.validate() {
            self.errors = FieldErrors::from_rules(&errors);
            return Ok(ActionResult::render());
        }
        let user = current_user().await?;
        // An empty text is no text, as Laravel's `ConvertEmptyStringsToNull`
        // makes it.
        let body = Some(input.body).filter(|body| !body.trim().is_empty());
        <Note as Model>::create(attrs! {
            user_id: user.id,
            title: input.title,
            body: body,
        })
        .await
        .map_err(|error| failed("saving a note", error))?;
        note_stats::publish_refresh(user.id).await;
        redirect("notes.index", Some(Toast::success("Note saved.")))
    }
}
