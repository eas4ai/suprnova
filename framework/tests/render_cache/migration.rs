//! The RenderCache migrations over tables that already exist.
//!
//! An app whose database already holds the RenderCache tables and their
//! indexes (created by an earlier run, or by hand) runs `up` again.
//! sea-query drops `IF NOT EXISTS` from `CREATE INDEX` on MySQL, so before
//! the framework's index guard that run failed there on the first existing
//! index (error 1061). The guard checks the catalogue the same way on every
//! backend, so this SQLite test covers the path MySQL takes.

use sea_orm::Database;
use sea_orm_migration::{MigrationTrait, SchemaManager};
use suprnova::render_cache::migration::{Migration, TierMigration};

#[tokio::test]
async fn up_over_existing_render_cache_tables_and_indexes_succeeds() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let manager = SchemaManager::new(&db);
    Migration
        .up(&manager)
        .await
        .expect("create the ledger tables and their index");
    TierMigration
        .up(&manager)
        .await
        .expect("create the tier tables and their indexes");

    Migration
        .up(&manager)
        .await
        .expect("up over existing ledger tables and their index");
    TierMigration
        .up(&manager)
        .await
        .expect("up over existing tier tables and their indexes");

    for (table, index) in [
        (
            "suprnova_render_generation_log",
            "suprnova_render_generation_log_identity",
        ),
        (
            "suprnova_render_entries",
            "idx_suprnova_render_entries_expires",
        ),
        (
            "suprnova_live_instances",
            "idx_suprnova_live_instances_expires",
        ),
        (
            "suprnova_live_promotions",
            "idx_suprnova_live_promotions_expires",
        ),
    ] {
        assert!(
            manager.has_index(table, index).await.unwrap(),
            "{table}.{index} is still there"
        );
    }
}
