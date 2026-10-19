//! Concurrent consumes of one auth-flow token on MariaDB and MySQL: the
//! token is spent once, and every other consumer is told it lost the race.
//!
//! `TokenStore::consume` reads the token's owner, then spends the token with
//! an UPDATE conditional on `used_at IS NULL`; that UPDATE's row count is the
//! single-use authority. MariaDB 11.8 and later run REPEATABLE READ with
//! `innodb_snapshot_isolation` on, where the UPDATE of a row another
//! transaction committed after the read fails with error 1020 instead of
//! matching zero rows, so a losing consumer saw a database error, and email
//! verification and password reset answered it with a 500. The framework
//! turns the setting off on every connection it opens, which gives MariaDB
//! MySQL's behavior.
//!
//! The overlap is made certain: a second connection holds the token's row
//! locked, every consumer reads the owner and then waits on the row, and the
//! holder lets go only once the server reports all of them waiting.
//!
//! Run against disposable databases:
//!
//! ```text
//! MARIADB_TEST_URL=mysql://... cargo test -p suprnova --test auth_flows -- \
//!   --ignored --test-threads=1 token_consume_engines::auth_flow_token_consume_concurrently_on_mariadb
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth_flows -- \
//!   --ignored --test-threads=1 token_consume_engines::auth_flow_token_consume_concurrently_on_mysql
//! ```

use std::time::Duration;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, TransactionTrait,
};
use serial_test::serial;
use suprnova::auth_flows::create_auth_flow_tokens_table;
use suprnova::auth_flows::token_store::{TokenPurpose, TokenStore};
use suprnova::testing::TestContainer;
use suprnova::{DatabaseConfig, DbConnection};

const CONSUMERS: usize = 6;

/// Counts the other sessions of this database whose spend of an auth-flow
/// token is running, which with the row held means waiting on its lock.
const WAITING_CONSUMERS: &str = "SELECT COUNT(*) FROM information_schema.PROCESSLIST \
     WHERE DB = DATABASE() AND ID <> CONNECTION_ID() \
     AND INFO LIKE 'UPDATE %auth_flow_tokens%'";

fn url(variable: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| panic!("set {variable} to a disposable database"))
}

/// A connection of its own, outside the framework's pool.
async fn connect(url: &str) -> DatabaseConnection {
    let mut options = ConnectOptions::new(url.to_owned());
    options
        .max_connections(1)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(10))
        .sqlx_logging(false);
    Database::connect(options)
        .await
        .expect("the test database must be reachable")
}

async fn execute(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

async fn waiting_consumers(watcher: &DatabaseConnection) -> i64 {
    let row = watcher
        .query_one_raw(Statement::from_string(
            watcher.get_database_backend(),
            WAITING_CONSUMERS,
        ))
        .await
        .unwrap_or_else(|error| panic!("{WAITING_CONSUMERS}: {error}"))
        .expect("a count returns a row");
    row.try_get_by_index::<i64>(0).expect("read the count")
}

/// Waits until every consumer has read the token and waits on its row.
/// Fails the test if that never happens.
async fn until_every_consumer_waits(watcher: &DatabaseConnection) {
    for _ in 0..1_000 {
        if waiting_consumers(watcher).await >= CONSUMERS as i64 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the consumers never all waited on the held token row");
}

async fn consume_concurrently(url: &str) {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(CONSUMERS as u32 + 2)
        .min_connections(0)
        .logging(false)
        .build();
    let pool = DbConnection::connect(&config)
        .await
        .expect("the framework's pool connects to the test database");
    execute(pool.inner(), "DROP TABLE IF EXISTS auth_flow_tokens").await;
    pool.inner()
        .execute(&create_auth_flow_tokens_table())
        .await
        .expect("create auth_flow_tokens");
    let guard = TestContainer::fake();
    TestContainer::singleton(pool.clone());

    let token = TokenStore::issue(
        "7",
        TokenPurpose::EmailVerification,
        chrono::Duration::hours(1),
    )
    .await
    .expect("issue a token");

    let holder = connect(url).await;
    let watcher = connect(url).await;
    let held = holder.begin().await.expect("begin the holder");
    held.execute_unprepared("SELECT id FROM auth_flow_tokens FOR UPDATE")
        .await
        .expect("hold the token's row");

    let consumers: Vec<_> = (0..CONSUMERS)
        .map(|_| {
            let token = token.clone();
            tokio::spawn(async move {
                TokenStore::consume(&token, TokenPurpose::EmailVerification).await
            })
        })
        .collect();
    until_every_consumer_waits(&watcher).await;
    held.commit().await.expect("let go of the token's row");

    let mut spent = 0;
    for (consumer, handle) in consumers.into_iter().enumerate() {
        match handle.await.expect("a consumer must not panic") {
            Ok(Some(owner)) => {
                assert_eq!(owner, "7", "the token belongs to user 7");
                spent += 1;
            }
            Ok(None) => {}
            Err(error) => panic!(
                "consumer {consumer} must be told it lost the race, not get an error: {error}"
            ),
        }
    }
    assert_eq!(spent, 1, "exactly one consumer spends the token");
    assert!(
        !TokenStore::check(&token, TokenPurpose::EmailVerification)
            .await
            .expect("check the token"),
        "the spent token is no longer live"
    );

    holder.close().await.expect("close the holder");
    watcher.close().await.expect("close the watcher");
    execute(pool.inner(), "DROP TABLE IF EXISTS auth_flow_tokens").await;
    drop(guard);
    pool.inner().close_by_ref().await.expect("close the pool");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB at MARIADB_TEST_URL"]
async fn auth_flow_token_consume_concurrently_on_mariadb() {
    consume_concurrently(&url("MARIADB_TEST_URL")).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn auth_flow_token_consume_concurrently_on_mysql() {
    consume_concurrently(&url("MYSQL_TEST_URL")).await;
}
