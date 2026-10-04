//! The schema builder cases against a real Postgres.
//!
//! Run them against a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test schema postgres_ -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, Value,
};
use serial_test::serial;

use super::{cases, laravel_cases};

pub(super) async fn connect_postgres() -> DatabaseConnection {
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

#[cfg(feature = "testing")]
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_native_timestamps_round_trip() {
    cases::native_timestamps_round_trip(&connect_postgres().await).await;
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

/// Postgres refuses a text parameter for a native date-time column. That is
/// why the default RFC 3339 cast cannot write such a column and a native
/// column needs `AsNativeDateTime` or `AsNaiveDateTime`, and why the owner
/// touch now binds the owner's cast instead of text.
#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_refuses_text_for_a_native_date_time_column() {
    let conn = connect_postgres().await;
    for sql in [
        "DROP TABLE IF EXISTS schema_text_into_native",
        "CREATE TABLE schema_text_into_native (at timestamp with time zone, naive timestamp)",
    ] {
        conn.execute_unprepared(sql).await.expect(sql);
    }
    for column in ["at", "naive"] {
        let insert = Statement::from_sql_and_values(
            DbBackend::Postgres,
            format!("INSERT INTO schema_text_into_native ({column}) VALUES ($1)"),
            [Value::String(Some("2031-03-14T09:00:00+00:00".to_string()))],
        );
        assert!(
            conn.execute_raw(insert).await.is_err(),
            "Postgres took text for the {column} column"
        );
    }
    conn.execute_unprepared("DROP TABLE schema_text_into_native")
        .await
        .expect("drop schema_text_into_native");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_laravel_column_types() {
    laravel_cases::laravel_column_types(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_primary_keys() {
    laravel_cases::primary_keys(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_foreign_on_a_declared_column() {
    laravel_cases::foreign_on_a_declared_column(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_laravel_misuse_is_refused() {
    laravel_cases::laravel_misuse_is_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_alter_laravel_additions() {
    laravel_cases::alter_laravel_additions(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_laravel_alter_misuse_is_refused() {
    laravel_cases::laravel_alter_misuse_is_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_action_shorthands() {
    laravel_cases::action_shorthands(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_unsigned_keys_everywhere() {
    laravel_cases::unsigned_keys_everywhere(&connect_postgres().await).await;
}
