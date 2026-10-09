use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;

/// Creates the `notes` table behind the notes page: each note belongs to
/// one user, who is the only one any page shows it to.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A notes table that already exists, including one a Laravel
        // application created, is left as it is.
        if manager.has_table("notes").await? {
            return Ok(());
        }
        Schema::create(manager, "notes", |t| {
            // `BIGINT UNSIGNED` on MySQL, as Laravel's `id()` creates it;
            // the `Note` model reads it into a `u64`.
            t.unsigned_id();
            // The owner. `users.id` is unsigned on MySQL, and a foreign key
            // there must match the sign of the column it references. Every
            // query the notes page runs filters on this column, so it gets
            // an index on every database (MySQL adds one for the key; Postgres
            // and SQLite do not). Deleting a user deletes their notes.
            t.unsigned_foreign_id("user_id")
                .constrained("users")
                .cascade_on_delete()
                .index();
            t.string("title");
            t.text("body").nullable();
            // Nullable DATETIME columns holding the UTC wall clock, as the
            // users table's are, read through the same casts.
            t.date_time("created_at").precision(0).nullable();
            t.date_time("updated_at").precision(0).nullable();
        })
        .await
    }

    /// Leaves the table. `up` skips a `notes` table that already exists,
    /// so the table may be an application's, with its notes, and rolling
    /// back must not drop them.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
