//! Note model.
//!
//! A note a user writes on the notes page. Each note belongs to one user
//! (`user_id`); the user model's `notes` relation is the other side.
//! Handlers read notes only through [`Note::owned_by`], so no page lists or
//! shows a note to anyone but the user who wrote it.

use chrono::{DateTime, Utc};
use suprnova::{Builder, Model, model};

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
pub struct Note {
    // `BIGINT UNSIGNED` on MySQL, as Laravel's `id()` creates it.
    pub id: u64,
    pub user_id: u64,
    pub title: String,
    // `NULL` when the note has a title and nothing else.
    pub body: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

// Re-export the SeaORM types the macro emits in the inner `note` module, as
// the `User` model does.
pub use note::{ActiveModel, Column, Entity};

impl Note {
    /// The notes `user_id` wrote, and no others.
    ///
    /// Every handler that reads notes starts here, so a note of another
    /// user is never listed, and asking for one by id finds nothing: the
    /// same `404` as an id that does not exist.
    pub fn owned_by(user_id: u64) -> Builder<Self> {
        <Self as Model>::query().filter("user_id", user_id)
    }
}
