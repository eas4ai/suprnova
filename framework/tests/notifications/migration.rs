//! #134: the framework ships the `notifications` table as a SeaORM
//! migration an app registers in its own `Migrator`. These tests run the
//! migration's `up` and `down` directly, the way a `Migrator` would.

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;
use suprnova::notifications::migrations::{
    CreateNotificationsTable, NotificationTimestampsToDatetime,
};

async fn sqlite() -> DatabaseConnection {
    Database::connect("sqlite::memory:").await.unwrap()
}

async fn index_names(db: &DatabaseConnection) -> Vec<String> {
    db.query_all_raw(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        "SELECT name FROM sqlite_master \
         WHERE type = 'index' AND tbl_name = 'notifications' AND name LIKE 'idx_%' \
         ORDER BY name"
            .to_string(),
    ))
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get_by_index::<String>(0).unwrap())
    .collect()
}

#[tokio::test]
async fn the_migration_has_a_stable_dated_name() {
    assert_eq!(
        CreateNotificationsTable.name(),
        "m20260516_000001_create_notifications_table"
    );
}

#[tokio::test]
async fn the_upgrade_migration_has_a_stable_dated_name() {
    assert_eq!(
        NotificationTimestampsToDatetime.name(),
        "m20261004_000001_notifications_timestamps_to_datetime"
    );
}

/// The upgrade only alters MySQL and MariaDB tables. On SQLite it leaves
/// the table, its columns and its rows alone, runs again harmlessly, and
/// its `down` changes nothing either.
#[tokio::test]
async fn the_upgrade_changes_nothing_outside_mysql() {
    let db = sqlite().await;
    let manager = SchemaManager::new(&db);
    CreateNotificationsTable.up(&manager).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO notifications \
         (id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at) \
         VALUES ('00000000-0000-0000-0000-000000000001', 'OrderShipped', 'users', '42', \
         '{}', NULL, '2040-06-01 12:00:00', '2040-06-01 12:00:00')",
    )
    .await
    .expect("store a row");
    let schema = || async {
        db.query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT sql FROM sqlite_master WHERE name = 'notifications'".to_string(),
        ))
        .await
        .unwrap()
        .expect("the table exists")
        .try_get_by_index::<String>(0)
        .unwrap()
    };
    let before = schema().await;

    NotificationTimestampsToDatetime.up(&manager).await.unwrap();
    NotificationTimestampsToDatetime
        .up(&manager)
        .await
        .expect("a second up is harmless");
    NotificationTimestampsToDatetime
        .down(&manager)
        .await
        .unwrap();

    assert_eq!(schema().await, before, "the table is unchanged");
    let created_at: String = db
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT created_at FROM notifications".to_string(),
        ))
        .await
        .unwrap()
        .expect("the row survives")
        .try_get_by_index(0)
        .unwrap();
    assert_eq!(created_at, "2040-06-01 12:00:00");
}

#[tokio::test]
async fn running_the_migration_twice_is_harmless() {
    let db = sqlite().await;
    let manager = SchemaManager::new(&db);

    CreateNotificationsTable.up(&manager).await.unwrap();
    CreateNotificationsTable
        .up(&manager)
        .await
        .expect("a second up finds the table and indexes and leaves them alone");

    assert!(manager.has_table("notifications").await.unwrap());
    assert_eq!(
        index_names(&db).await,
        vec!["idx_notifications_notifiable", "idx_notifications_read_at"]
    );
}

/// The schema the framework shipped as raw SQL before this migration
/// existed, which the manual told apps to apply by hand. The statements
/// are plain SQL that SQLite, Postgres and MySQL all accept.
const HAND_MADE_SCHEMA: [&str; 3] = [
    "CREATE TABLE notifications (\
        id CHAR(36) PRIMARY KEY, \
        type VARCHAR(255) NOT NULL, \
        notifiable_type VARCHAR(255) NOT NULL, \
        notifiable_id VARCHAR(64) NOT NULL, \
        data TEXT NOT NULL, \
        read_at TIMESTAMP NULL, \
        created_at TIMESTAMP NOT NULL, \
        updated_at TIMESTAMP NOT NULL\
     )",
    "CREATE INDEX idx_notifications_notifiable ON notifications(notifiable_type, notifiable_id)",
    "CREATE INDEX idx_notifications_read_at ON notifications(read_at)",
];

/// #134 review: an app that created the table by hand registers the
/// migration and runs `up`. The table and both indexes already exist, so
/// `up` must leave them, and the row in them, alone - twice. Before the
/// index guard, MySQL failed here on the first index (error 1061), which
/// blocked every later migration. Each backend's test calls this on an
/// empty database.
pub(crate) async fn up_over_the_hand_made_schema_keeps_it(db: &DatabaseConnection) {
    for sql in HAND_MADE_SCHEMA {
        db.execute_unprepared(sql)
            .await
            .expect("apply the hand-made schema");
    }
    db.execute_unprepared(
        "INSERT INTO notifications \
         (id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at) \
         VALUES ('00000000-0000-0000-0000-000000000134', 'OrderShipped', 'users', '42', \
         '{}', NULL, '2026-05-16 00:00:00', '2026-05-16 00:00:00')",
    )
    .await
    .expect("store a row under the hand-made schema");
    let manager = SchemaManager::new(db);

    CreateNotificationsTable
        .up(&manager)
        .await
        .expect("up over a hand-made table and its indexes");
    CreateNotificationsTable
        .up(&manager)
        .await
        .expect("a second up is harmless too");

    for index in ["idx_notifications_notifiable", "idx_notifications_read_at"] {
        assert!(
            manager.has_index("notifications", index).await.unwrap(),
            "{index} is still there"
        );
    }
    let rows: i64 = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT COUNT(*) AS n FROM notifications".to_string(),
        ))
        .await
        .unwrap()
        .expect("a count row")
        .try_get("", "n")
        .unwrap();
    assert_eq!(rows, 1, "the stored notification survives");
}

#[tokio::test]
async fn up_over_a_hand_made_table_and_indexes_succeeds() {
    up_over_the_hand_made_schema_keeps_it(&sqlite().await).await;
}

#[tokio::test]
async fn down_drops_the_table() {
    let db = sqlite().await;
    let manager = SchemaManager::new(&db);
    CreateNotificationsTable.up(&manager).await.unwrap();

    CreateNotificationsTable.down(&manager).await.unwrap();

    assert!(!manager.has_table("notifications").await.unwrap());
    assert!(index_names(&db).await.is_empty());
}
