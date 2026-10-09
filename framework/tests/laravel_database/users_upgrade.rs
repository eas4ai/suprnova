use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::DbBackend;
use suprnova::schema::Schema;

/// Moves the `users` table an earlier scaffold created into the Laravel 13
/// skeleton's layout. Every row keeps its id. A run that stops part way
/// starts again from `users_earlier` on the next `migrate`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        if !manager.has_table("users_earlier").await? {
            db.execute_unprepared("CREATE TABLE users_earlier AS SELECT * FROM users")
                .await?;
        }
        Schema::drop_if_exists(manager, "users").await?;
        Schema::create(manager, "users", |t| {
            t.unsigned_id();
            t.string("name");
            t.string("email").unique();
            t.date_time("email_verified_at").precision(0).nullable();
            t.string("password");
            t.remember_token();
            t.date_time("created_at").precision(0).nullable();
            t.date_time("updated_at").precision(0).nullable();
        })
        .await?;
        db.execute_unprepared(
            "INSERT INTO users (id, name, email, email_verified_at, password, \
             remember_token, created_at, updated_at) \
             SELECT id, name, email, email_verified_at, password, remember_token, \
             created_at, updated_at FROM users_earlier",
        )
        .await?;
        Schema::drop_if_exists(manager, "users_earlier").await?;
        if manager.get_database_backend() == DbBackend::Postgres {
            // The rows kept their ids; the next one follows the largest.
            db.execute_unprepared(
                "SELECT setval(pg_get_serial_sequence('users', 'id'), \
                 COALESCE((SELECT MAX(id) FROM users), 0) + 1, false)",
            )
            .await?;
        }
        Ok(())
    }

    /// Leaves the skeleton's layout, which holds every earlier row.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
