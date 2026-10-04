//! PostgreSQL coverage for the `database` notification channel and the
//! notification read model (DATA-01).
//!
//! Every statement in the write channel and the read helpers was
//! hand-written with `?` positional placeholders - SQLite/MySQL syntax that
//! Postgres rejects - so persisting or reading a notification failed
//! outright on Postgres. The rest of the notification suite runs on
//! in-memory SQLite, which is why the breakage was invisible.
//!
//! Run with a disposable Postgres:
//!
//! ```text
//! docker run -d --rm --name suprnova-pg -e POSTGRES_PASSWORD=pw \
//!     -e POSTGRES_DB=suprnova_test -p 55999:5432 postgres:17-alpine
//! PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55999/suprnova_test \
//!     cargo test -p suprnova --test notification_database_postgres -- --ignored
//! ```

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::notifications::channels::database::DatabaseChannel;
use suprnova::notifications::migrations::CreateNotificationsTable;
use suprnova::notifications::{
    Channel, Notifiable, Notification, NotificationDispatcher, all_for, delete_for,
    mark_all_as_read, mark_as_read, mark_as_unread, read_for, unread_for,
};

async fn connect_postgres() -> DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(4)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

/// The table name is fixed by the migration, so every test in this binary
/// shares it - hence `#[serial]` plus a drop-and-recreate per test.
///
/// The shipped migration runs through `MigrationTrait::up` rather than a
/// `Migrator`: a `Migrator` records what it applied in `seaql_migrations`,
/// so after the first test it would skip the table this one just dropped,
/// and the gate's Postgres database is shared with other test binaries
/// whose own migrators record there too.
async fn fresh_db() -> DatabaseConnection {
    let db = connect_postgres().await;
    recreate_table(&db).await;
    db
}

/// Drop `notifications` and create it again with the shipped migration.
/// Shared with the MySQL/MariaDB run in `database_mysql`.
pub(crate) async fn recreate_table(db: &DatabaseConnection) {
    db.execute_unprepared("DROP TABLE IF EXISTS notifications")
        .await
        .expect("drop notifications");
    CreateNotificationsTable
        .up(&SchemaManager::new(db))
        .await
        .expect("apply notifications migration");
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OrderShipped {
    tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }
    fn channels(&self) -> Vec<&'static str> {
        vec!["database"]
    }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User {
    id: i64,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        (channel == "database").then(|| self.id.to_string())
    }
}

async fn seed_n(db: &DatabaseConnection, user_id: i64, n: usize) -> Vec<String> {
    let channel: Arc<dyn Channel> = Arc::new(DatabaseChannel::new(db.clone(), "users"));
    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    for i in 0..n {
        dispatcher
            .notify(
                &User { id: user_id },
                &OrderShipped {
                    tracking: format!("T{i}"),
                },
            )
            .await
            .expect("deliver notification");
    }
    all_for(db, "users", &user_id.to_string())
        .await
        .expect("read back")
        .iter()
        .map(|r| r.id.clone())
        .collect()
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_channel_writes_rows_the_read_model_can_load() {
    channel_writes_rows_the_read_model_can_load(&fresh_db().await).await;
}

pub(crate) async fn channel_writes_rows_the_read_model_can_load(db: &DatabaseConnection) {
    let ids = seed_n(db, 1, 3).await;
    assert_eq!(ids.len(), 3);

    let rows = all_for(db, "users", "1").await.expect("all_for");
    assert_eq!(rows.len(), 3);
    for r in &rows {
        assert_eq!(r.notifiable_type, "users");
        assert_eq!(r.notifiable_id, "1");
        assert_eq!(r.type_name, "OrderShipped");
        assert!(r.read_at.is_none());
        assert!(r.data.get("tracking").is_some(), "data column round-trips");
    }
    assert!(rows[0].created_at >= rows[2].created_at, "newest first");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_mark_read_unread_and_partitioned_reads() {
    mark_read_unread_and_partitioned_reads(&fresh_db().await).await;
}

pub(crate) async fn mark_read_unread_and_partitioned_reads(db: &DatabaseConnection) {
    let ids = seed_n(db, 2, 3).await;

    mark_as_read(db, &ids[0]).await.expect("mark_as_read");
    mark_as_read(db, &ids[0]).await.expect("idempotent");

    let unread = unread_for(db, "users", "2").await.expect("unread_for");
    let read = read_for(db, "users", "2").await.expect("read_for");
    assert_eq!(unread.len(), 2);
    assert_eq!(read.len(), 1);
    assert!(read[0].read_at.is_some());
    assert!(
        read[0].read_at.unwrap() >= read[0].created_at,
        "read_at decodes as the time it was marked"
    );

    mark_as_unread(db, &ids[0]).await.expect("mark_as_unread");
    mark_as_unread(db, &ids[0]).await.expect("idempotent");
    assert_eq!(unread_for(db, "users", "2").await.unwrap().len(), 3);
    assert_eq!(read_for(db, "users", "2").await.unwrap().len(), 0);
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_mark_all_as_read_and_delete_for_report_row_counts() {
    mark_all_as_read_and_delete_for_report_row_counts(&fresh_db().await).await;
}

pub(crate) async fn mark_all_as_read_and_delete_for_report_row_counts(db: &DatabaseConnection) {
    seed_n(db, 3, 4).await;
    // A second recipient must be untouched by both mass operations.
    seed_n(db, 4, 2).await;

    let updated = mark_all_as_read(db, "users", "3")
        .await
        .expect("mark_all_as_read");
    assert_eq!(updated, 4);
    assert_eq!(unread_for(db, "users", "3").await.unwrap().len(), 0);
    assert_eq!(unread_for(db, "users", "4").await.unwrap().len(), 2);

    let deleted = delete_for(db, "users", "3").await.expect("delete_for");
    assert_eq!(deleted, 4);
    assert_eq!(all_for(db, "users", "3").await.unwrap().len(), 0);
    assert_eq!(all_for(db, "users", "4").await.unwrap().len(), 2);
}

/// #134 review: `CreateNotificationsTable` run over a table and indexes an
/// app created by hand leaves them, and their rows, alone.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_up_over_a_hand_made_table_and_indexes_succeeds() {
    let db = connect_postgres().await;
    db.execute_unprepared("DROP TABLE IF EXISTS notifications")
        .await
        .expect("drop notifications");

    crate::migration::up_over_the_hand_made_schema_keeps_it(&db).await;

    db.execute_unprepared("DROP TABLE notifications")
        .await
        .expect("drop notifications");
}
