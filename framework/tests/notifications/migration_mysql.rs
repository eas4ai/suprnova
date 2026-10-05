//! MySQL/MariaDB coverage for `CreateNotificationsTable` over a table an
//! app created by hand. sea-query drops `IF NOT EXISTS` from `CREATE
//! INDEX` on MySQL, so this is the backend where `up` used to fail with
//! error 1061 on the first existing index.
//!
//! Ignored in the normal suite. Run it against a disposable database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test notifications -- \
//!   --ignored --test-threads=1 migration_mysql::
//! ```
//!
//! Explicit execution without `MYSQL_TEST_URL` fails immediately; it never
//! reports a silent pass.

use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::notifications::migrations::{
    CreateNotificationsTable, NotificationTimestampsToDatetime,
};

async fn connect_mysql() -> DatabaseConnection {
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

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_up_over_a_hand_made_table_and_indexes_succeeds() {
    let db = connect_mysql().await;
    db.execute_unprepared("DROP TABLE IF EXISTS notifications")
        .await
        .expect("drop notifications");

    crate::migration::up_over_the_hand_made_schema_keeps_it(&db).await;

    db.execute_unprepared("DROP TABLE notifications")
        .await
        .expect("drop notifications");
}

/// A connection to `MYSQL_TEST_URL` whose session time zone is `zone`
/// rather than the driver's default UTC.
async fn connect_in_zone(zone: &str) -> DatabaseConnection {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database");
    let separator = if url.contains('?') { '&' } else { '?' };
    let zone = zone.replace('+', "%2B");
    let mut options = ConnectOptions::new(format!("{url}{separator}timezone={zone}"));
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("MariaDB/MySQL test database must be reachable")
}

/// Every notification migration the framework ships, in order, as an
/// app's `Migrator` lists them.
async fn run_shipped_migrations(db: &DatabaseConnection) {
    let manager = SchemaManager::new(db);
    CreateNotificationsTable
        .up(&manager)
        .await
        .expect("create the notifications table");
    NotificationTimestampsToDatetime
        .up(&manager)
        .await
        .expect("move the time columns to DATETIME");
}

/// Type and nullability of each time column, in the table's order.
async fn time_columns(db: &DatabaseConnection) -> Vec<(String, String, String)> {
    db.query_all_raw(Statement::from_string(
        db.get_database_backend(),
        "SELECT COLUMN_NAME AS name, DATA_TYPE AS kind, IS_NULLABLE AS nullable \
         FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'notifications' \
         AND COLUMN_NAME IN ('read_at', 'created_at', 'updated_at') \
         ORDER BY ORDINAL_POSITION",
    ))
    .await
    .expect("read the time columns")
    .iter()
    .map(|row| {
        let get = |column: &str| -> String {
            row.try_get::<String>("", column)
                .expect("a column attribute")
                .to_ascii_lowercase()
        };
        (get("name"), get("kind"), get("nullable"))
    })
    .collect()
}

/// The stored times of every row as text, in UTC wall-clock form.
async fn stored_times(db: &DatabaseConnection) -> Vec<(String, Option<String>, String, String)> {
    db.query_all_raw(Statement::from_string(
        db.get_database_backend(),
        "SELECT id, CAST(read_at AS CHAR) AS read_at, CAST(created_at AS CHAR) AS created_at, \
         CAST(updated_at AS CHAR) AS updated_at FROM notifications ORDER BY id",
    ))
    .await
    .expect("read the stored times")
    .iter()
    .map(|row| {
        (
            row.try_get::<String>("", "id").expect("id"),
            row.try_get::<Option<String>>("", "read_at")
                .expect("read_at"),
            row.try_get::<String>("", "created_at").expect("created_at"),
            row.try_get::<String>("", "updated_at").expect("updated_at"),
        )
    })
    .collect()
}

/// A `notifications` table an older version of the migration created keeps
/// MySQL's `TIMESTAMP`, which refuses any time after 2038-01-19: from then
/// on every notification write fails. The shipped migrations convert it to
/// `DATETIME`, keep each column's nullability and every stored time in UTC,
/// and run again harmlessly. They run here over a session in another time
/// zone, which `ALTER TABLE` would otherwise convert the stored times into.
#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_the_shipped_migrations_move_a_timestamp_table_past_2038_in_utc() {
    let db = crate::database_mysql::legacy_table().await;
    db.execute_unprepared(
        "INSERT INTO notifications \
         (id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at) VALUES \
         ('00000000-0000-0000-0000-000000000001', 'OrderShipped', 'users', '1', '{}', \
          NULL, '2030-01-02 03:04:05', '2030-01-02 03:04:06'), \
         ('00000000-0000-0000-0000-000000000002', 'OrderShipped', 'users', '1', '{}', \
          '2031-05-06 07:08:09', '2031-05-06 07:08:00', '2031-05-06 07:08:09')",
    )
    .await
    .expect("store rows in the legacy table");
    let before = stored_times(&db).await;

    let shifted = connect_in_zone("+05:00").await;
    run_shipped_migrations(&shifted).await;
    run_shipped_migrations(&shifted).await;

    assert_eq!(
        time_columns(&db).await,
        vec![
            (
                "read_at".to_owned(),
                "datetime".to_owned(),
                "yes".to_owned()
            ),
            (
                "created_at".to_owned(),
                "datetime".to_owned(),
                "no".to_owned()
            ),
            (
                "updated_at".to_owned(),
                "datetime".to_owned(),
                "no".to_owned()
            ),
        ],
        "every time column is DATETIME, with its nullability kept"
    );
    assert_eq!(
        stored_times(&db).await,
        before,
        "every stored time is the same UTC time"
    );
    db.execute_unprepared(
        "INSERT INTO notifications \
         (id, type, notifiable_type, notifiable_id, data, read_at, created_at, updated_at) VALUES \
         ('00000000-0000-0000-0000-000000000003', 'OrderShipped', 'users', '1', '{}', \
          NULL, '2040-06-01 12:00:00', '2040-06-01 12:00:00')",
    )
    .await
    .expect("a time after 2038 fits");

    db.execute_unprepared("DROP TABLE notifications")
        .await
        .expect("drop notifications");
}
