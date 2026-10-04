//! MySQL/MariaDB coverage for the `database` notification channel and the
//! notification read model, over the table the shipped migration creates.
//!
//! The migration creates `read_at`, `created_at` and `updated_at` with
//! `.timestamp()`, which is `TIMESTAMP` on MySQL and MariaDB. The read
//! model decoded them as `NaiveDateTime`, which the MySQL driver decodes
//! only from `DATETIME`, so every read failed there, and `read_at` read
//! back as "unread" whatever it held.
//!
//! Ignored in the normal suite. Run it against a disposable database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test notifications -- \
//!   --ignored --test-threads=1 database_mysql::
//! ```

use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};
use serial_test::serial;

use crate::database_postgres::{
    channel_writes_rows_the_read_model_can_load, mark_all_as_read_and_delete_for_report_row_counts,
    mark_read_unread_and_partitioned_reads, recreate_table,
};

/// A connection with `notifications` freshly created by the shipped
/// migration, after checking the migration made its time columns
/// `TIMESTAMP`: the type this file exists to cover.
async fn fresh_db() -> DatabaseConnection {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    let db = Database::connect(options)
        .await
        .expect("MariaDB/MySQL test database must be reachable");
    recreate_table(&db).await;
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
    assert_eq!(kind.to_ascii_lowercase(), "timestamp");
    db
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_channel_writes_rows_the_read_model_can_load() {
    channel_writes_rows_the_read_model_can_load(&fresh_db().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_mark_read_unread_and_partitioned_reads() {
    mark_read_unread_and_partitioned_reads(&fresh_db().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_mark_all_as_read_and_delete_for_report_row_counts() {
    mark_all_as_read_and_delete_for_report_row_counts(&fresh_db().await).await;
}
