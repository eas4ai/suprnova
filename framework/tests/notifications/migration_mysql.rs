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

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use serial_test::serial;

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
