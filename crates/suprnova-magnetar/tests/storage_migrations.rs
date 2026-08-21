#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::migrations::sqlite;
use storage_schema::{StorageSchema, database};

#[tokio::test]
async fn guarded_lookup_indexes_are_replay_safe() {
    let db = database().await;
    let first = sqlite::apply::<StorageSchema>(&db).await.unwrap();
    let second = sqlite::apply::<StorageSchema>(&db).await.unwrap();
    assert_eq!(first.statements, 2);
    assert_eq!(second.statements, 0);
}

#[tokio::test]
async fn missing_role_columns_skip_index_creation() {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let db = Database::connect("sqlite::memory:").await.unwrap();
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TABLE storage_tokens (id INTEGER PRIMARY KEY, digest TEXT NOT NULL, purpose TEXT NOT NULL)",
    ))
    .await
    .unwrap();
    let report = sqlite::apply::<StorageSchema>(&db).await.unwrap();
    assert_eq!(report.statements, 0);
}
