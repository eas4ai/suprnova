use sea_orm_migration::MigratorTrait;

mod m20240101_000001_create_users_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        let mut migrations: Vec<Box<dyn sea_orm_migration::MigrationTrait>> = vec![
            Box::new(m20240101_000001_create_users_table::Migration),
            // The browser sessions of the sign-in page the approval and
            // claim pages send a guest to (`controllers::sign_in`), in the
            // layout Laravel 13's skeleton creates.
            Box::new(suprnova::session::migrations::CreateSessionsTable::new(
                suprnova::session::migrations::SessionUserKey::Integer,
            )),
        ];
        // The OAuth server's clients, grants and tokens, and agent auth's
        // registrations and claims (`src/agents.rs`).
        migrations.extend(suprnova::agents::migrations());
        migrations
    }
}
