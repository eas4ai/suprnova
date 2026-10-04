//! The second-factor attempt counter on PostgreSQL and MySQL.
//!
//! The counter serializes reservations with a row lock on the user's
//! `two_factor_credentials` row, and the SQLite suite cannot prove a row
//! lock: SQLite locks the whole database. These tests run the reservation
//! path against real engines with a connection pool, so parallel requests
//! genuinely contend for the lock.
//!
//! Run explicitly against disposable databases, one test at a time:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test auth_flows -- --ignored --test-threads=1 two_factor_engines::postgres_
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test auth_flows -- --ignored --test-threads=1 two_factor_engines::mysql_
//! ```
//!
//! Without the variable an explicitly selected test fails at once; it never
//! reports a silent pass. The tests share one database and one of them drops
//! and recreates the attempt table, so they must run one at a time.

use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use sea_orm_migration::prelude::{MigrationTrait, SchemaManager};
use serial_test::serial;
use suprnova::auth_flows::two_factor::migration::Migration as TwoFactorMigration;
use suprnova::auth_flows::two_factor::migration_attempts::Migration as TwoFactorAttemptsMigration;
use suprnova::auth_flows::two_factor::migration_replay::Migration as TwoFactorReplayMigration;
use suprnova::auth_flows::{TwoFactor, TwoFactorUser};
use suprnova::testing::{TestContainer, TestContainerGuard};
use suprnova::{Crypt, DbConnection, EncryptionKey};

/// The default threshold of the counter.
const THRESHOLD: usize = 5;

#[derive(Clone, Copy)]
enum Engine {
    Postgres,
    Mysql,
}

struct EngineUser {
    id: String,
    email: String,
}

impl TwoFactorUser for EngineUser {
    fn user_id(&self) -> &str {
        &self.id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

async fn connect(engine: Engine) -> DatabaseConnection {
    let url = match engine {
        Engine::Postgres => {
            std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres")
        }
        Engine::Mysql => std::env::var("MYSQL_TEST_URL")
            .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database"),
    };
    let mut options = ConnectOptions::new(url);
    // Enough connections for every parallel request to hold its own, so the
    // requests contend for the row lock rather than for the pool.
    options
        .max_connections(10)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(10))
        .sqlx_logging(false);
    Database::connect(options)
        .await
        .expect("the test database must be reachable")
}

/// Run `sql` over the text protocol: MySQL refuses `CREATE TRIGGER` as a
/// prepared statement.
async fn execute(conn: &DatabaseConnection, sql: &str) {
    conn.execute_unprepared(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

/// Create the two-factor tables. PostgreSQL runs the framework migrations.
/// On MySQL the credentials migration's TEXT primary key cannot be indexed,
/// so that table is created by hand in the same shape, and the attempt
/// migration runs as shipped.
async fn create_tables(conn: &DatabaseConnection, engine: Engine) {
    let manager = SchemaManager::new(conn);
    match engine {
        Engine::Postgres => {
            if !manager
                .has_table("two_factor_credentials")
                .await
                .expect("catalogue")
            {
                TwoFactorMigration.up(&manager).await.expect("credentials");
                TwoFactorReplayMigration
                    .up(&manager)
                    .await
                    .expect("replay column");
            }
        }
        Engine::Mysql => {
            execute(
                conn,
                "CREATE TABLE IF NOT EXISTS two_factor_credentials (\
                    user_id VARCHAR(255) NOT NULL PRIMARY KEY, \
                    secret TEXT NOT NULL, \
                    confirmed_at TIMESTAMP NULL, \
                    recovery_codes TEXT NULL, \
                    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
                    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
                    last_used_timestep BIGINT NULL\
                 )",
            )
            .await;
        }
    }
    TwoFactorAttemptsMigration
        .up(&manager)
        .await
        .expect("attempt table");
}

/// Install the engine for this test, and an enrolled, confirmed user with an
/// id of its own.
async fn enrolled(
    engine: Engine,
    label: &str,
) -> (
    TestContainerGuard,
    DatabaseConnection,
    EngineUser,
    suprnova::auth_flows::EnrollmentResponse,
) {
    if !Crypt::is_initialized() {
        Crypt::init(EncryptionKey::generate());
    }
    let conn = connect(engine).await;
    create_tables(&conn, engine).await;
    let guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let user = EngineUser {
        id: format!("{label}-{}", uuid::Uuid::new_v4().simple()),
        email: format!("{label}@example.test"),
    };
    let resp = TwoFactor::enroll(&user).await.expect("enroll");
    TwoFactor::confirm(&user, &totp_code_for(&resp.otpauth_url))
        .await
        .expect("confirm");
    (guard, conn, user, resp)
}

fn totp_code_for(otpauth_url: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    let url = url::Url::parse(otpauth_url).expect("otpauth url");
    let secret = url
        .query_pairs()
        .find(|(key, _)| key == "secret")
        .map(|(_, value)| value.into_owned())
        .expect("secret query parameter");
    let bytes = Secret::Encoded(secret).to_bytes().expect("decode secret");
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, "user".into())
        .expect("totp")
        .generate_current()
        .expect("generate code")
}

/// Status of one verify outcome: 200 accepted, 401 rejected, or the error.
fn status(outcome: &Result<bool, suprnova::FrameworkError>) -> u16 {
    match outcome {
        Ok(true) => 200,
        Ok(false) => 401,
        Err(error) => error.status_code(),
    }
}

/// Make every attempt reservation hold its transaction open a little
/// longer, so parallel admissions overlap on the server even when the
/// statements themselves are fast. Without the row lock they would all
/// count before any of them commits.
async fn slow_down_reservations(conn: &DatabaseConnection, engine: Engine) {
    match engine {
        Engine::Postgres => {
            execute(
                conn,
                "CREATE OR REPLACE FUNCTION two_factor_attempts_slow_insert() RETURNS trigger \
                 AS $$ BEGIN PERFORM pg_sleep(0.05); RETURN NEW; END $$ LANGUAGE plpgsql",
            )
            .await;
            execute(
                conn,
                "CREATE TRIGGER two_factor_attempts_slow_insert BEFORE INSERT ON \
                 two_factor_attempts FOR EACH ROW EXECUTE FUNCTION \
                 two_factor_attempts_slow_insert()",
            )
            .await;
        }
        Engine::Mysql => {
            execute(
                conn,
                "CREATE TRIGGER two_factor_attempts_slow_insert BEFORE INSERT ON \
                 two_factor_attempts FOR EACH ROW SET @two_factor_attempts_slow = SLEEP(0.05)",
            )
            .await;
        }
    }
}

async fn restore_reservation_speed(conn: &DatabaseConnection, engine: Engine) {
    match engine {
        Engine::Postgres => {
            execute(
                conn,
                "DROP TRIGGER IF EXISTS two_factor_attempts_slow_insert ON two_factor_attempts",
            )
            .await;
            execute(
                conn,
                "DROP FUNCTION IF EXISTS two_factor_attempts_slow_insert()",
            )
            .await;
        }
        Engine::Mysql => {
            execute(
                conn,
                "DROP TRIGGER IF EXISTS two_factor_attempts_slow_insert",
            )
            .await;
        }
    }
}

/// Eight parallel wrong codes: exactly the threshold is evaluated, every
/// other request is refused before its code is read, and nothing fails for
/// want of the lock.
async fn parallel_wrong_codes_evaluate_at_most_the_threshold(engine: Engine) {
    let (_guard, conn, user, _resp) = enrolled(engine, "parallel-wrong").await;
    slow_down_reservations(&conn, engine).await;

    let guesses = (0..THRESHOLD + 3).map(|_| TwoFactor::verify(&user, "000000"));
    let statuses: Vec<u16> = futures::future::join_all(guesses)
        .await
        .iter()
        .map(status)
        .collect();
    restore_reservation_speed(&conn, engine).await;

    assert_eq!(
        statuses.iter().filter(|status| **status == 401).count(),
        THRESHOLD,
        "only the threshold of guesses is evaluated: {statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|status| **status == 429).count(),
        3,
        "every guess past it is refused: {statuses:?}"
    );
}

/// Four parallel requests submit the same correct code: one is accepted,
/// the others lose the timestep claim, and nothing fails for want of the
/// lock.
async fn a_correct_code_under_contention_is_accepted_once(engine: Engine) {
    let (_guard, _conn, user, resp) = enrolled(engine, "contended-code").await;
    let code = totp_code_for(&resp.otpauth_url);

    let attempts = (0..4).map(|_| TwoFactor::verify(&user, &code));
    let statuses: Vec<u16> = futures::future::join_all(attempts)
        .await
        .iter()
        .map(status)
        .collect();

    assert_eq!(
        statuses.iter().filter(|status| **status == 200).count(),
        1,
        "one code, one acceptance: {statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|status| **status == 401).count(),
        3,
        "the others are rejected, not failed: {statuses:?}"
    );
}

/// The attempt migration runs over an existing attempt table, as every
/// framework migration must when an app registers it over a schema that
/// already has it.
async fn the_attempt_migration_runs_over_an_existing_table(engine: Engine) {
    let conn = connect(engine).await;
    create_tables(&conn, engine).await;
    TwoFactorAttemptsMigration
        .up(&SchemaManager::new(&conn))
        .await
        .expect("a second run over the existing table and index succeeds");
}

/// Without the attempt table every proof answers 503, and the log names
/// the migration to add, in this engine's wording of a missing table.
async fn a_missing_attempt_table_logs_the_migration_to_add(engine: Engine) {
    let (_guard, conn, user, resp) = enrolled(engine, "missing-table").await;
    execute(&conn, "DROP TABLE two_factor_attempts").await;

    let outcome = TwoFactor::verify(&user, &totp_code_for(&resp.otpauth_url)).await;

    // Recreate the table before asserting, so a failure here does not take
    // the table away from the other tests.
    TwoFactorAttemptsMigration
        .up(&SchemaManager::new(&conn))
        .await
        .expect("recreate the attempt table");
    assert_eq!(status(&outcome), 503);
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_parallel_wrong_codes_evaluate_at_most_the_threshold() {
    parallel_wrong_codes_evaluate_at_most_the_threshold(Engine::Postgres).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_a_correct_code_under_contention_is_accepted_once() {
    a_correct_code_under_contention_is_accepted_once(Engine::Postgres).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_the_attempt_migration_runs_over_an_existing_table() {
    the_attempt_migration_runs_over_an_existing_table(Engine::Postgres).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
#[tracing_test::traced_test]
async fn postgres_a_missing_attempt_table_logs_the_migration_to_add() {
    a_missing_attempt_table_logs_the_migration_to_add(Engine::Postgres).await;
    assert!(logs_contain("migration_attempts"));
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_parallel_wrong_codes_evaluate_at_most_the_threshold() {
    parallel_wrong_codes_evaluate_at_most_the_threshold(Engine::Mysql).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_a_correct_code_under_contention_is_accepted_once() {
    a_correct_code_under_contention_is_accepted_once(Engine::Mysql).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_the_attempt_migration_runs_over_an_existing_table() {
    the_attempt_migration_runs_over_an_existing_table(Engine::Mysql).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
#[tracing_test::traced_test]
async fn mysql_a_missing_attempt_table_logs_the_migration_to_add() {
    a_missing_attempt_table_logs_the_migration_to_add(Engine::Mysql).await;
    assert!(logs_contain("migration_attempts"));
}
