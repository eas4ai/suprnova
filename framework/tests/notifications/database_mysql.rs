//! MySQL/MariaDB coverage for the `database` notification channel and the
//! notification read model, over the table the shipped migration creates
//! and over the one older versions of it created.
//!
//! The migration once created `read_at`, `created_at` and `updated_at`
//! with `.timestamp()`, which is `TIMESTAMP` on MySQL and MariaDB. The read
//! model decoded them as `NaiveDateTime`, which the MySQL driver decodes
//! only from `DATETIME`, so every read failed there, and `read_at` read
//! back as "unread" whatever it held. MySQL also refuses to store a
//! `TIMESTAMP` after 2038-01-19, so every notification written after that
//! would fail. The migration now creates `DATETIME`; tables an older
//! migration created keep `TIMESTAMP` and still read.
//!
//! Ignored in the normal suite. Run it against a disposable database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test notifications -- \
//!   --ignored --test-threads=1 database_mysql::
//! ```

use std::time::Duration;

use chrono::{TimeZone, Utc};
use sea_orm::sea_query::{ColumnDef, Table};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DeriveIden, Statement,
};
use sea_orm_migration::SchemaManager;
use serial_test::serial;
use suprnova::testing::TestClock;

use crate::database_postgres::{
    channel_writes_rows_the_read_model_can_load, mark_all_as_read_and_delete_for_report_row_counts,
    mark_read_unread_and_partitioned_reads, recreate_table,
};

async fn connect() -> DatabaseConnection {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("MariaDB/MySQL test database must be reachable")
}

/// The type MySQL reports for `notifications.created_at`.
async fn created_at_type(db: &DatabaseConnection) -> String {
    let row = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT DATA_TYPE AS kind FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'notifications' \
             AND COLUMN_NAME = 'created_at'",
        ))
        .await
        .expect("read column type")
        .expect("created_at exists");
    let kind: String = row.try_get("", "kind").expect("column type");
    kind.to_ascii_lowercase()
}

/// A connection with `notifications` freshly created by the shipped
/// migration.
async fn shipped_table() -> DatabaseConnection {
    let db = connect().await;
    recreate_table(&db).await;
    db
}

#[derive(DeriveIden)]
enum Notifications {
    Table,
    Id,
    Type,
    NotifiableType,
    NotifiableId,
    Data,
    ReadAt,
    CreatedAt,
    UpdatedAt,
}

/// A connection with `notifications` as older versions of the migration
/// created it: every time column `.timestamp()`.
async fn legacy_table() -> DatabaseConnection {
    let db = connect().await;
    db.execute_unprepared("DROP TABLE IF EXISTS notifications")
        .await
        .expect("drop notifications");
    SchemaManager::new(&db)
        .create_table(
            Table::create()
                .table(Notifications::Table)
                .col(
                    ColumnDef::new(Notifications::Id)
                        .char_len(36)
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Notifications::Type)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Notifications::NotifiableType)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Notifications::NotifiableId)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(Notifications::Data).text().not_null())
                .col(ColumnDef::new(Notifications::ReadAt).timestamp().null())
                .col(
                    ColumnDef::new(Notifications::CreatedAt)
                        .timestamp()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Notifications::UpdatedAt)
                        .timestamp()
                        .not_null(),
                )
                .to_owned(),
        )
        .await
        .expect("create the legacy notifications table");
    assert_eq!(created_at_type(&db).await, "timestamp");
    db
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_the_migration_creates_datetime_columns() {
    let db = shipped_table().await;
    assert_eq!(created_at_type(&db).await, "datetime");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_channel_writes_rows_the_read_model_can_load() {
    channel_writes_rows_the_read_model_can_load(&shipped_table().await).await;
    channel_writes_rows_the_read_model_can_load(&legacy_table().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_mark_read_unread_and_partitioned_reads() {
    mark_read_unread_and_partitioned_reads(&shipped_table().await).await;
    mark_read_unread_and_partitioned_reads(&legacy_table().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_mark_all_as_read_and_delete_for_report_row_counts() {
    mark_all_as_read_and_delete_for_report_row_counts(&shipped_table().await).await;
    mark_all_as_read_and_delete_for_report_row_counts(&legacy_table().await).await;
}

/// A notification written after 2038-01-19, when MySQL's `TIMESTAMP` can
/// no longer hold the current time, still persists and reads back.
#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_a_notification_written_after_2038_reads_back() {
    let _clock = TestClock::travel_to(
        Utc.with_ymd_and_hms(2040, 6, 1, 12, 0, 0)
            .single()
            .expect("valid date"),
    );
    channel_writes_rows_the_read_model_can_load(&shipped_table().await).await;
}
