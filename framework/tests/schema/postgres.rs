//! The schema builder cases against a real Postgres.
//!
//! Run them against a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test schema postgres_ -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use serial_test::serial;

use super::cases;

async fn connect_postgres() -> DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_every_column_type() {
    cases::every_column_type(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_model_round_trip() {
    cases::model_round_trip(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_foreign_key_cascade() {
    cases::foreign_key_cascade(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_alter_foreign_key() {
    cases::alter_foreign_key(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_unique_index_refuses_duplicate() {
    cases::unique_index_refuses_duplicate(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_alter_indexes() {
    cases::alter_indexes(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_alter_columns() {
    cases::alter_columns(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_tables_drop_and_rename() {
    cases::tables_drop_and_rename(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_misuse_is_refused_before_any_statement() {
    cases::misuse_is_refused_before_any_statement(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_migrator_runs_both_styles() {
    cases::migrator_runs_both_styles(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_undeclared_index_column_is_refused() {
    cases::undeclared_index_column_is_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_duplicate_added_column_is_refused() {
    cases::duplicate_added_column_is_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_quoted_string_default_round_trips() {
    cases::quoted_string_default_round_trips(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_a_name_longer_than_the_limit_is_refused() {
    cases::a_name_longer_than_the_limit_is_refused(&connect_postgres().await).await;
}
