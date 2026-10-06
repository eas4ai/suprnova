//! Framework-owned migrations for the feature-flags subsystem.
//!
//! Re-exported under [`crate::features::migrations`] so consumer apps
//! can register the schema in their own `Migrator`:
//!
//! ```rust,no_run
//! use suprnova::features::migrations::{
//!     CreateFeaturesTable, FeatureTimestampsToDatetime, FeaturesToPennantLayout,
//! };
//! # struct Migrator;
//!
//! impl sea_orm_migration::MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![
//!             Box::new(CreateFeaturesTable),
//!             Box::new(FeatureTimestampsToDatetime),
//!             Box::new(FeaturesToPennantLayout),
//!         ]
//!     }
//! }
//! ```
//!
//! The `features` table is laravel/pennant's. [`FeatureTimestampsToDatetime`]
//! moves a table an older version created on MySQL or MariaDB from
//! `TIMESTAMP` to `DATETIME` time columns, and [`FeaturesToPennantLayout`]
//! moves it into Pennant's layout with its flags. A table Pennant created
//! is left alone by all three.

pub mod m_create_features_table;
pub mod m_features_pennant_layout;
pub mod m_features_timestamps_to_datetime;

/// Public alias so consumers can write `CreateFeaturesTable` instead of
/// the date-prefixed module name. The actual migration name on the
/// `seaql_migrations` table comes from
/// [`MigrationName::name`](sea_orm_migration::MigrationName::name), not
/// the type ident, so this alias is purely an ergonomic re-export.
pub use m_create_features_table::Migration as CreateFeaturesTable;

/// Public alias for the upgrade that moves a `features` table an older
/// [`CreateFeaturesTable`] created on MySQL or MariaDB from `TIMESTAMP` to
/// `DATETIME` time columns. Apps list it right after [`CreateFeaturesTable`].
pub use m_features_timestamps_to_datetime::Migration as FeatureTimestampsToDatetime;

/// Public alias for the upgrade that moves a `features` table an older
/// [`CreateFeaturesTable`] created into laravel/pennant's layout, with its
/// flags. Apps list it after [`FeatureTimestampsToDatetime`].
pub use m_features_pennant_layout::Migration as FeaturesToPennantLayout;

/// Whether `columns` are the earlier Suprnova layout of `features`, which
/// keyed a flag by `scope_key`; Pennant's names the column `scope`.
pub(crate) fn is_earlier_layout(columns: &[crate::database::catalog::CatalogColumn]) -> bool {
    crate::database::catalog::column(columns, "scope_key").is_some()
}
