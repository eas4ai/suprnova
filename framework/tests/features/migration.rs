//! The framework's features migration over a table that already exists.
//!
//! An app whose database already holds `features` and its unique index
//! (created by an earlier run, or by hand) runs `up` again. The index had
//! no `IF NOT EXISTS`, so that run failed on every backend. It now goes
//! through the framework's index guard, which checks the catalogue the
//! same way on every backend, so this SQLite test covers MySQL's path too.

use sea_orm::Database;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::features::migrations::CreateFeaturesTable;

#[tokio::test]
async fn up_over_an_existing_features_table_and_index_succeeds() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    CreateFeaturesTable
        .up(&manager)
        .await
        .expect("create the table and its index");

    CreateFeaturesTable
        .up(&manager)
        .await
        .expect("up over an existing features table and its index");

    assert!(
        manager
            .has_index("features", "idx_features_name_scope_key")
            .await
            .unwrap(),
        "the unique index is still there"
    );
}
