//! Note model.
//!
//! A note a user writes on the notes page. Each note belongs to one user
//! (`user_id`); the user model's `notes` relation is the other side.
//! Ordinary reads use [`Note::owned_by`]. Search declares an owner scope
//! over `user_id`, so every search path hides notes from other viewers.

use chrono::{DateTime, Utc};
use suprnova::search::{AttributeValue, Filter, Viewer};
use suprnova::{Builder, FrameworkError, Model, Searchable, model};

#[model(
    table = "notes",
    fillable = ["user_id", "title", "body"],
    timestamps,
    // The notes migration creates these as nullable DATETIME columns
    // holding the UTC wall clock, as the users table's are, so they take
    // the same casts as the `User` model's.
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
    // `user()` reaches the owner through `user_id`, the column the
    // convention names for a `BelongsTo<User>`.
    relations = {
        user: BelongsTo<crate::models::user::User>,
    },
)]
/// The owner scope makes the same private note projection safe for every search path.
#[derive(Searchable)]
#[searchable(index = "notes", scope = "own_notes", scope_fields("user_id"))]
pub struct Note {
    // `BIGINT UNSIGNED` on MySQL, as Laravel's `id()` creates it.
    pub id: u64,
    #[searchable(filterable)]
    pub user_id: u64,
    #[searchable(text)]
    pub title: String,
    // `NULL` when the note has a title and nothing else.
    #[searchable(text)]
    pub body: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Note {
    /// Provisions the initial generation before writes without replacing a live index.
    pub async fn initialize_search() -> Result<(), FrameworkError> {
        let engine = suprnova::search::SearchRegistry::engine_for("notes")?;
        if engine.current_generation("notes").await?.is_some() {
            return Ok(());
        }
        match suprnova::search::generations::create_index("notes").await {
            Ok(()) => Ok(()),
            Err(error) => {
                // Another process can publish the same initial index during boot.
                if engine.current_generation("notes").await?.is_some() {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    /// The notes `user_id` wrote, and no others.
    ///
    /// Every handler that reads notes starts here, so a note of another
    /// user is never listed, and asking for one by id finds nothing: the
    /// same `404` as an id that does not exist.
    pub fn owned_by(user_id: u64) -> Builder<Self> {
        <Self as Model>::query().filter("user_id", user_id)
    }
}

// Guests receive an impossible owner id; every search path uses this scope.
fn own_notes(viewer: &Viewer) -> Filter {
    let owner = viewer
        .id()
        .and_then(|id| id.parse::<i64>().ok())
        .unwrap_or(-1);
    Filter::Eq("user_id".to_owned(), AttributeValue::Integer(owner))
}
