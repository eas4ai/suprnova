//! `NoteStats`: the dashboard's counts of the user's notes and the five
//! newest, which refresh without a page load.
//!
//! The island subscribes to the `notes` stream on the topic
//! `notes/:principal`, which Live fills with the signed-in user's id. Saving
//! a note publishes a refresh on that topic ([`publish_refresh`]), and every
//! dashboard the user has open renders again from the server, over
//! server-sent events or a WebSocket, falling back to polling when neither
//! connects. The numbers are read on every render, so a refresh, a poll and
//! the Refresh button all show what the database holds now.

use chrono::Utc;
use suprnova::live::{ComponentError, EventPayloadMetadata, LiveComponent, LiveStreams, live};

use super::note_list::NoteRow;
use super::{current_user, failed};
use crate::models::note::Note;

/// The checked loop-key filter the recent notes use.
pub mod filters {
    pub use suprnova::view::filters::live_key;
}

/// How many notes the dashboard lists.
const RECENT_NOTES: u64 = 5;

/// The event contract the `notes` stream carries. The kit publishes
/// refreshes on the stream; publish this with `LiveStreams::event` to reach
/// a browser listener of your own.
pub struct NotesChanged;

impl EventPayloadMetadata for NotesChanged {
    const NAME: &'static str = "notes.changed";
    const VERSION: u16 = 1;
}

/// The topic of `user_id`'s notes stream, as `notes/:principal` resolves
/// for that user.
pub fn topic(user_id: u64) -> String {
    format!("notes/{user_id}")
}

/// Tells every dashboard `user_id` has open to render its numbers again.
///
/// A refresh that cannot be published is logged and skipped rather than
/// failing the save that asked for it: the note is saved, and the dashboard
/// catches up at its next poll or page load.
pub async fn publish_refresh(user_id: u64) {
    let published = match LiveStreams::resolve() {
        Ok(streams) => streams.refresh(&topic(user_id)).await,
        Err(error) => Err(error),
    };
    if let Err(error) = published {
        tracing::warn!(error = %error, "publishing a notes refresh failed");
    }
}

/// The counts and the newest notes, rendered by `live/note_stats.html`.
#[derive(LiveComponent)]
#[live(
    name = "{package_name}.note-stats",
    view = "live/note_stats.html",
    minimum_protocol_version = 2,
    streams(stream(name = "notes", topics("notes/:principal"), events(NotesChanged)))
)]
pub struct NoteStats {
    /// How many notes the user has.
    #[server_only]
    notes: i64,
    /// How many of them the user wrote today (the UTC day the server is in).
    #[server_only]
    written_today: i64,
    /// The user's newest notes, newest first.
    #[server_only]
    recent: Vec<NoteRow>,
}

#[live]
impl NoteStats {
    /// Reads the numbers and the newest notes for this render. The id
    /// orders the notes, since two notes written in the same second share a
    /// `created_at`.
    #[rendering]
    pub async fn load(&mut self) -> Result<(), ComponentError> {
        let user = current_user().await?;
        self.notes = Note::owned_by(user.id)
            .count()
            .await
            .map_err(|error| failed("counting notes", error))?;
        self.written_today = Note::owned_by(user.id)
            .where_date("created_at", Utc::now().date_naive())
            .count()
            .await
            .map_err(|error| failed("counting today's notes", error))?;
        self.recent = Note::owned_by(user.id)
            .latest_by("id")
            .limit(RECENT_NOTES)
            .get()
            .await
            .map_err(|error| failed("listing recent notes", error))?
            .into_vec()
            .into_iter()
            .map(NoteRow::from)
            .collect();
        Ok(())
    }

    /// Renders again, which reads the numbers again.
    #[action]
    pub fn refresh(&mut self) {}
}
