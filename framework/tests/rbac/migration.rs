//! The framework's RBAC migration over tables that already exist.
//!
//! An app whose database already holds the RBAC tables and their indexes
//! (created by an earlier run, by hand, or by spatie/laravel-permission)
//! runs `up` again. An earlier version's indexes had no `IF NOT EXISTS`, so
//! that run failed on every backend. The migration now leaves tables that
//! exist exactly as they are, and the same catalogue check decides that on
//! every backend, so this SQLite test covers MySQL's path too.

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
        ("roles", "roles_name_guard_name_unique"),
        ("permissions", "permissions_name_guard_name_unique"),
        ("model_has_roles", "model_has_roles_model_id_model_type_index"),
        (
            "model_has_permissions",
            "model_has_permissions_model_id_model_type_index",
        ),
    ] {
        assert!(
            manager.has_index(table, index).await.unwrap(),
            "{table}.{index} is still there"
        );
    }
}
