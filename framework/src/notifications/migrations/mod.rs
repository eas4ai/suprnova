//! Framework-owned migrations for the `database` notification channel.
//!
//! [`CreateNotificationsTable`] creates the `notifications` table that
//! [`DatabaseChannel`](crate::notifications::channels::database::DatabaseChannel)
//! writes and the inbox helpers in [`crate::notifications`] read, and
//! [`NotificationTimestampsToDatetime`] upgrades a table an older version of
//! it created on MySQL or MariaDB. Consumer apps register both, in that
//! order, in their own `Migrator`:
//!
//! ```rust,no_run
//! use sea_orm_migration::MigratorTrait;
//! use suprnova::notifications::migrations::{
//!     CreateNotificationsTable, NotificationTimestampsToDatetime,
//! };
//!
//! pub struct Migrator;
//!
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![
//!             // ... the app's own migrations ...
//!             Box::new(CreateNotificationsTable),
//!             Box::new(NotificationTimestampsToDatetime),
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
pub mod m_notifications_timestamps_to_datetime;

/// Public alias so consumers can write `CreateNotificationsTable` instead
/// of the module path. The name recorded in `seaql_migrations` comes from
/// [`MigrationName::name`](sea_orm_migration::MigrationName::name), not
/// the type ident.
pub use m_create_notifications_table::Migration as CreateNotificationsTable;

/// Public alias for the upgrade that moves a `notifications` table an
/// older [`CreateNotificationsTable`] created on MySQL or MariaDB from
/// `TIMESTAMP` to `DATETIME` time columns. Apps list it right after
/// [`CreateNotificationsTable`].
pub use m_notifications_timestamps_to_datetime::Migration as NotificationTimestampsToDatetime;
