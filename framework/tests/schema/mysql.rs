//! The schema builder cases against a real MariaDB or MySQL.
//!
//! Run them against a disposable database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test schema mysql_ -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use serial_test::serial;

use super::cases;

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
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_every_column_type() {
    cases::every_column_type(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_model_round_trip() {
    cases::model_round_trip(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_native_timestamps_round_trip() {
    cases::native_timestamps_round_trip(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_foreign_key_cascade() {
    cases::foreign_key_cascade(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_alter_foreign_key() {
    cases::alter_foreign_key(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_unique_index_refuses_duplicate() {
    cases::unique_index_refuses_duplicate(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_alter_indexes() {
    cases::alter_indexes(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_alter_columns() {
    cases::alter_columns(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_tables_drop_and_rename() {
    cases::tables_drop_and_rename(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_misuse_is_refused_before_any_statement() {
    cases::misuse_is_refused_before_any_statement(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_migrator_runs_both_styles() {
    cases::migrator_runs_both_styles(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_undeclared_index_column_is_refused() {
    cases::undeclared_index_column_is_refused(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_duplicate_added_column_is_refused() {
    cases::duplicate_added_column_is_refused(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_quoted_string_default_round_trips() {
    cases::quoted_string_default_round_trips(&connect_mysql().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_a_name_longer_than_the_limit_is_refused() {
    cases::a_name_longer_than_the_limit_is_refused(&connect_mysql().await).await;
}
