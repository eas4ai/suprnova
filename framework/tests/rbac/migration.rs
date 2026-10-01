//! The framework's RBAC migration over tables that already exist.
//!
//! An app whose database already holds the RBAC tables and their unique
//! indexes (created by an earlier run, or by hand) runs `up` again. The
//! indexes had no `IF NOT EXISTS`, so that run failed on every backend.
//! They now go through the framework's index guard, which checks the
//! catalogue the same way on every backend, so this SQLite test covers
//! MySQL's path too.

use sea_orm::Database;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::rbac::migrations::CreateRbacTables;

#[tokio::test]
async fn up_over_existing_rbac_tables_and_indexes_succeeds() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    CreateRbacTables
        .up(&manager)
        .await
        .expect("create the tables and their indexes");

    CreateRbacTables
        .up(&manager)
        .await
        .expect("up over existing RBAC tables and their indexes");

    for (table, index) in [
        ("roles", "idx_roles_name_guard_name"),
        ("permissions", "idx_permissions_name_guard_name"),
        ("role_permissions", "idx_role_permissions_role_permission"),
        ("model_roles", "idx_model_roles_model_role"),
        (
            "model_permissions",
            "idx_model_permissions_model_permission",
        ),
    ] {
        assert!(
            manager.has_index(table, index).await.unwrap(),
            "{table}.{index} is still there"
        );
    }
}
