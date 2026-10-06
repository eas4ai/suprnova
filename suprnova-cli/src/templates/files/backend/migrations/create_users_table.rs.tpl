use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The users table of the Laravel 13 skeleton, so this application
        // runs on a database a Laravel application created. A `users` table
        // that already exists, Laravel's included, is left as it is.
        if manager.has_table("users").await? {
            return Ok(());
        }
        Schema::create(manager, "users", |t| {
            // `BIGINT UNSIGNED` on MySQL, as Laravel's `id()` creates it;
            // the `User` model reads it into a `u64`.
            t.unsigned_id();
            t.string("name");
            t.string("email").unique();
            // The time columns are nullable, as Laravel's are, and
            // `.date_time()`: DATETIME on MySQL, `timestamp` on Postgres,
            // holding the UTC wall clock. The `User` model reads and writes
            // them through the `AsOptionalNaiveDateTime` casts, which also
            // read the `TIMESTAMP` columns Laravel's migration creates.
            t.date_time("email_verified_at").precision(0).nullable();
            t.string("password");
            t.remember_token();
            t.date_time("created_at").precision(0).nullable();
            t.date_time("updated_at").precision(0).nullable();
        })
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop_if_exists(manager, "users").await
    }
}
