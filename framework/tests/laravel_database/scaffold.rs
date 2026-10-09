//! A fresh scaffold's migrations and `User` model, taken from the
//! templates `suprnova new` writes, and the Migrator a scaffolded
//! application runs with every framework migration that creates or changes
//! a table whose name Laravel or a package in scope uses.

use sea_orm::DatabaseConnection;
use sea_orm_migration::{MigrationTrait, MigratorTrait};

// `#[rustfmt::skip]`: the templates are scaffold output, not workspace
// source, so `cargo fmt` must not rewrite them.
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_users_table.rs.tpl"]
pub mod m20240101_000001_create_users_table;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_sessions_table.rs.tpl"]
pub mod m20240101_000002_create_sessions_table;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_remember_tokens_table.rs.tpl"]
pub mod m20240101_000003_create_remember_tokens_table;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_auth_flow_tokens_table.rs.tpl"]
pub mod m20240101_000004_create_auth_flow_tokens_table;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/migrations/create_notes_table.rs.tpl"]
pub mod m20240101_000005_create_notes_table;
#[rustfmt::skip]
#[path = "../../../suprnova-cli/src/templates/files/backend/models/user.rs.tpl"]
pub mod user;

/// The scaffold's Migrator, then the framework's migrations for the tables
/// LDB-001 names.
pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        use suprnova::features::migrations::{
            CreateFeaturesTable, FeatureTimestampsToDatetime, FeaturesToPennantLayout,
        };
        use suprnova::notifications::migrations::{
            CreateNotificationsTable, NotificationTimestampsToDatetime,
            NotificationsToLaravelLayout,
        };
        use suprnova::queue::migrations::{
            CreateFailedJobsTable, CreateJobBatchesTable, CreateJobsTable,
        };
        use suprnova::rbac::migrations::{CreateRbacTables, RbacToSpatieLayout};
        use suprnova::session::migrations::{CreateSessionsTable, SessionUserKey};
        vec![
            Box::new(m20240101_000001_create_users_table::Migration),
            Box::new(m20240101_000002_create_sessions_table::Migration),
            Box::new(m20240101_000003_create_remember_tokens_table::Migration),
            Box::new(m20240101_000004_create_auth_flow_tokens_table::Migration),
            Box::new(m20240101_000005_create_notes_table::Migration),
            Box::new(CreateSessionsTable::new(SessionUserKey::Integer)),
            Box::new(suprnova::render_cache::migration::Migration),
            Box::new(CreateJobsTable),
            Box::new(CreateJobBatchesTable),
            Box::new(CreateFailedJobsTable),
            Box::new(CreateNotificationsTable),
            Box::new(NotificationTimestampsToDatetime),
            Box::new(NotificationsToLaravelLayout),
            Box::new(CreateFeaturesTable),
            Box::new(FeatureTimestampsToDatetime),
            Box::new(FeaturesToPennantLayout),
            Box::new(CreateRbacTables),
            Box::new(RbacToSpatieLayout),
        ]
    }
}

/// Run [`Migrator`] on `conn`.
pub async fn migrate(conn: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    Migrator::up(conn, None).await
}

/// Roll every migration in [`Migrator`] back in reverse order.
pub async fn rollback(conn: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
    Migrator::down(conn, None).await
}
