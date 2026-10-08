use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;

/// Creates the `notes` table behind the notes page: each note belongs to
/// one user, who is the only one any page shows it to.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
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

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop_if_exists(manager, "notes").await
    }
}
