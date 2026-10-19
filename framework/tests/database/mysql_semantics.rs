//! Every MySQL-family connection the framework opens runs with MySQL 8.4's
//! concurrency semantics, on MariaDB as well.
//!
//! MariaDB 11.8 and later turn `innodb_snapshot_isolation` on by default.
//! Under it, an UPDATE or DELETE in a REPEATABLE READ transaction fails
//! with error 1020 when another transaction committed a change to the row
//! after the snapshot, where MySQL waits and re-checks. The framework's
//! concurrency logic is written for MySQL's behavior, so each new pooled
//! connection turns the setting off for its own session. These tests open
//! each kind of pool the framework builds and read the session's value.
//!
//! Run against disposable databases:
//!
//! ```text
//! MARIADB_TEST_URL=mysql://... cargo test -p suprnova --test database -- \
//!   --ignored mysql_semantics::a_new_mariadb_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test database -- \
//!   --ignored mysql_semantics::a_new_mysql_
//! ```

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};
use sea_orm_migration::prelude::*;
use suprnova::testing::TestDatabase;
use suprnova::{DatabaseConfig, DbConnection, MariaDbVectorDriver};

use crate::env_snapshot::{EnvSnapshot, set_env};

/// MySQL's and MariaDB's `ER_UNKNOWN_SYSTEM_VARIABLE`.
const UNKNOWN_SYSTEM_VARIABLE: &str = "1193";

/// The statement that reads the session's setting.
const READ_SETTING: &str = "SELECT @@SESSION.innodb_snapshot_isolation";

/// No migration, and a migration table of its own, so an engine run never
/// races another test's ledger.
struct NoMigrations;

impl MigratorTrait for NoMigrations {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }

    fn migration_table_name() -> DynIden {
        Alias::new("suprnova_mysql_semantics_ledger").into_iden()
    }
}

fn url(variable: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| panic!("set {variable} to a disposable database"))
}

/// The session's `innodb_snapshot_isolation`, read on a connection the pool
/// opened for this call.
async fn setting(conn: &DatabaseConnection) -> Result<i64, DbErr> {
    let row = conn
        .query_one_raw(Statement::from_string(
            conn.get_database_backend(),
            READ_SETTING,
        ))
        .await?
        .expect("a SELECT of a variable returns one row");
    Ok(row.try_get_by_index::<i64>(0).expect("read the setting"))
}

/// The pools the framework builds for a MySQL-family URL: the primary or a
/// named connection, a test transaction, and the MariaDB vector driver's.
/// Each is asked for its session's setting on its first connection.
async fn settings_of_every_pool(url: &str) -> Vec<(&'static str, Result<i64, DbErr>)> {
    let mut settings = Vec::new();

    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(2)
        .min_connections(0)
        .logging(false)
        .build();
    let primary = DbConnection::connect(&config)
        .await
        .expect("the framework's pool connects");
    settings.push(("DbConnection::connect", setting(primary.inner()).await));
    primary
        .inner()
        .close_by_ref()
        .await
        .expect("close the pool");

    {
        let _env = EnvSnapshot::capture(&["DATABASE_URL"]);
        set_env("DATABASE_URL", Some(url));
        let db = TestDatabase::refresh::<NoMigrations>()
            .await
            .expect("hold a test transaction");
        settings.push(("TestDatabase::refresh", setting(db.conn()).await));
    }

    let vectors = MariaDbVectorDriver::from_url(url).expect("the vector pool builds");
    let mut connection = vectors.pool().acquire().await.expect("a vector connection");
    let read: Result<(i64,), _> = sea_orm::sqlx::query_as(READ_SETTING)
        .fetch_one(&mut *connection)
        .await;
    settings.push((
        "MariaDbVectorDriver::from_url",
        read.map(|(value,)| value)
            .map_err(|error| DbErr::Custom(error.to_string())),
    ));
    drop(connection);
    vectors.pool().close().await;

    settings
}

#[tokio::test]
#[ignore = "requires disposable MariaDB at MARIADB_TEST_URL"]
async fn a_new_mariadb_connection_runs_without_snapshot_isolation() {
    let _env = crate::env_lock::lock_env_async().await;
    for (pool, setting) in settings_of_every_pool(&url("MARIADB_TEST_URL")).await {
        let setting = setting.unwrap_or_else(|error| {
            panic!("{pool}: MariaDB 11.8 and later know innodb_snapshot_isolation: {error}")
        });
        assert_eq!(
            setting, 0,
            "{pool}: a new connection must run with innodb_snapshot_isolation off"
        );
    }
}

/// MySQL has no such variable. The connection the framework opens runs
/// anyway: the statement that turns the setting off is refused as unknown,
/// and that refusal means there is nothing to turn off.
#[tokio::test]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn a_new_mysql_connection_runs_without_snapshot_isolation() {
    let _env = crate::env_lock::lock_env_async().await;
    for (pool, setting) in settings_of_every_pool(&url("MYSQL_TEST_URL")).await {
        let error = setting.expect_err("MySQL has no innodb_snapshot_isolation");
        assert!(
            error.to_string().contains(UNKNOWN_SYSTEM_VARIABLE),
            "{pool}: the pool opened a connection, and only the variable is unknown: {error}"
        );
    }
}
