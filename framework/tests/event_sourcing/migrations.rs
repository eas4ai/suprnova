use super::*;
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

async fn constraint(db: &sea_orm::DatabaseConnection) {
    let id = uuid();
    let backend = db.get_database_backend();
    let placeholders = if backend == DbBackend::Postgres {
        "$1, 1, 1, $2, $3, $4, $5"
    } else {
        "?, 1, 1, ?, ?, ?, ?"
    };
    let statement = Statement::from_sql_and_values(
        backend,
        format!(
            "INSERT INTO stored_events (aggregate_uuid, aggregate_version, event_version, event_type, payload, metadata, created_at) VALUES ({placeholders})"
        ),
        vec![
            id.clone().into(),
            "es.added".into(),
            serde_json::json!({"amount":1}).into(),
            serde_json::json!({}).into(),
            chrono::Utc::now().naive_utc().into(),
        ],
    );
    db.execute_raw(statement.clone()).await.unwrap();
    assert!(db.execute_raw(statement).await.is_err());
    let placeholders = if backend == DbBackend::Postgres {
        "$1"
    } else {
        "?"
    };
    db.execute_raw(Statement::from_sql_and_values(
        backend,
        format!("DELETE FROM stored_events WHERE aggregate_uuid = {placeholders}"),
        [id.into()],
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn es_002_sqlite_tables_and_unique_constraint() {
    let db = database().await;
    constraint(db.conn()).await;
    let manager = SchemaManager::new(db.conn());
    for table in ["stored_events", "snapshots", "projector_checkpoints"] {
        assert!(manager.has_table(table).await.unwrap());
    }
    suprnova::CreateEventSourcingTables
        .down(&manager)
        .await
        .unwrap();
    assert!(!manager.has_table("stored_events").await.unwrap());
}

async fn engine(variable: &str) {
    let db = Database::connect(std::env::var(variable).expect(variable))
        .await
        .unwrap();
    let manager = SchemaManager::new(&db);
    if !manager.has_table("stored_events").await.unwrap() {
        suprnova::CreateEventSourcingTables
            .up(&manager)
            .await
            .unwrap();
    }
    for table in ["stored_events", "snapshots", "projector_checkpoints"] {
        assert!(manager.has_table(table).await.unwrap());
    }
    constraint(&db).await;
}
#[tokio::test]
#[ignore = "requires PG_TEST_URL"]
async fn es_002_postgres_tables_and_unique_constraint() {
    engine("PG_TEST_URL").await;
}
#[tokio::test]
#[ignore = "requires MYSQL_TEST_URL"]
async fn es_002_mysql_tables_and_unique_constraint() {
    engine("MYSQL_TEST_URL").await;
}
