//! Framework-owned migrations for role-based access control.
//!
//! The tables take spatie/laravel-permission's layout, so roles,
//! permissions and assignments spatie recorded apply in a Suprnova
//! application on the same database. [`CreateRbacTables`] creates what is
//! missing, and [`RbacToSpatieLayout`] moves the tables an earlier release
//! created into that layout with their rows. A table spatie created is left
//! alone by both. Applications list them in that order:
//!
//! ```rust,no_run
//! use sea_orm_migration::MigratorTrait;
//! use suprnova::rbac::migrations::{CreateRbacTables, RbacToSpatieLayout};
//!
//! pub struct Migrator;
//!
//! impl MigratorTrait for Migrator {
//!     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
//!         vec![Box::new(CreateRbacTables), Box::new(RbacToSpatieLayout)]
//!     }
//! }
//! ```

pub mod m_create_rbac_tables;
pub mod m_rbac_spatie_layout;

/// Public alias for the RBAC table migration used by consumer apps.
pub use m_create_rbac_tables::Migration as CreateRbacTables;

/// Public alias for the upgrade that moves the RBAC tables an earlier
/// [`CreateRbacTables`] created into spatie/laravel-permission's layout,
/// with their rows. Apps list it right after [`CreateRbacTables`].
pub use m_rbac_spatie_layout::Migration as RbacToSpatieLayout;

/// Whether the RBAC tables are still in the earlier Suprnova layout: a
/// `roles` or `permissions` with `display_name`, or an earlier assignment
/// table.
pub(crate) async fn is_earlier_layout(
    manager: &sea_orm_migration::SchemaManager<'_>,
) -> Result<bool, sea_orm_migration::DbErr> {
    for table in ["roles", "permissions"] {
        let columns =
            crate::database::catalog::table_columns(manager.get_connection(), table).await?;
        if crate::database::catalog::column(&columns, "display_name").is_some() {
            return Ok(true);
        }
    }
    for table in ["model_roles", "model_permissions", "role_permissions"] {
        if manager.has_table(table).await? {
            return Ok(true);
        }
    }
    Ok(false)
}
