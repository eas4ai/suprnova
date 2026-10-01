//! Framework-owned migration for the `database` notification channel.
//!
//! It creates the `notifications` table that
//! [`DatabaseChannel`](crate::notifications::channels::database::DatabaseChannel)
//! writes and the inbox helpers in [`crate::notifications`] read.
//! Consumer apps register it in their own `Migrator`:
//!
//! ```rust,no_run
//! use sea_orm_migration::MigratorTrait;
//! use suprnova::notifications::migrations::CreateNotificationsTable;
//!
//! pub struct Migrator;
//!
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![
//!             // ... the app's own migrations ...
//!             Box::new(CreateNotificationsTable),
//!         ]
//!     }
//! }
//! ```
//!
//! The framework owns the schema, the app owns when to apply it:
//! `suprnova migrate` creates the table once the app's `Migrator` lists
//! the migration.
//!
//! Matches the convention used by [`crate::workflow::migrations`],
//! [`crate::features::migrations`] and [`crate::payments::migrations`].

pub mod m_create_notifications_table;

/// Public alias so consumers can write `CreateNotificationsTable` instead
/// of the module path. The name recorded in `seaql_migrations` comes from
/// [`MigrationName::name`](sea_orm_migration::MigrationName::name), not
/// the type ident.
pub use m_create_notifications_table::Migration as CreateNotificationsTable;
