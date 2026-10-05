//! IDENTITY-023 on PostgreSQL, MariaDB and MySQL: email verification stamps
//! only the address the link was mailed to, even when a second connection
//! changes the address while `verify` runs.
//!
//! `EloquentUserProvider::mark_email_verified_for` rereads the user with
//! `SELECT ... FOR UPDATE` in the transaction that writes the stamp. SQLite
//! has no row locks, so only a real engine proves that lock. A second
//! connection holds the address change uncommitted; `verify` reaches the
//! locked reread and waits on it; the holder commits only once the server
//! reports that wait. The reread must then see the new address and refuse.
//!
//! Run explicitly against disposable databases, one test at a time:
//!
//! ```text
//! PG_TEST_URL=postgres://... cargo test -p suprnova --test auth_flows -- \
//!   --ignored --test-threads=1 email_verify_engines::postgres_
//! MYSQL_TEST_URL=mysql://... cargo test -p suprnova --test auth_flows -- \
//!   --ignored --test-threads=1 email_verify_engines::mysql_
//! ```

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement, TransactionTrait,
};
use serial_test::serial;
use suprnova::auth::AuthConfig;
use suprnova::auth_flows::EmailVerification;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthManager, Authenticatable, CanResetPassword, DbConnection, EloquentUserProvider,
    MustVerifyEmail, UserProvider, model,
};

/// The verification stamp is cast native: the column is a native timestamp
/// on every engine, which refuses the default text binding.
#[model(
    table = "ev_users",
    fillable = ["email", "password"],
    casts = { email_verified_at = suprnova::AsOptionalNativeDateTime },
)]
pub struct EvUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl Authenticatable for EvUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for EvUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
        self.email_verified_at = v;
    }
}

impl CanResetPassword for EvUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

#[derive(Clone, Copy)]
enum Engine {
    Postgres,
    Mysql,
}

impl Engine {
    fn url(self) -> String {
        match self {
            Self::Postgres => {
                std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres")
            }
            Self::Mysql => std::env::var("MYSQL_TEST_URL")
                .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database"),
        }
    }

    /// The `ev_users` table, with the engine's native timestamp.
    fn create_users(self) -> &'static str {
        match self {
            Self::Postgres => {
                "CREATE TABLE ev_users (id BIGSERIAL PRIMARY KEY, email VARCHAR(255) NOT NULL, \
                 password VARCHAR(255) NOT NULL, email_verified_at TIMESTAMPTZ NULL)"
            }
            Self::Mysql => {
                "CREATE TABLE ev_users (id BIGINT AUTO_INCREMENT PRIMARY KEY, \
                 email VARCHAR(255) NOT NULL, password VARCHAR(255) NOT NULL, \
                 email_verified_at TIMESTAMP NULL)"
            }
        }
    }

    /// Counts the sessions of this database whose statement on `ev_users`
    /// waits on a row lock. The holder's finished update is idle. On MySQL
    /// the probe's own text would match its pattern, so its connection is
    /// left out.
    fn waiting_probe(self) -> &'static str {
        match self {
            Self::Postgres => {
                "SELECT COUNT(*) FROM pg_stat_activity \
                 WHERE datname = current_database() AND wait_event_type = 'Lock' \
                 AND query LIKE '%ev_users%'"
            }
            Self::Mysql => {
                "SELECT COUNT(*) FROM information_schema.PROCESSLIST \
                 WHERE DB = DATABASE() AND ID <> CONNECTION_ID() \
                 AND (INFO LIKE 'SELECT %ev_users%FOR UPDATE%' OR INFO LIKE 'UPDATE %ev_users%')"
            }
        }
    }
}

async fn connect(engine: Engine, connections: u32) -> DatabaseConnection {
    let mut options = ConnectOptions::new(engine.url());
    options
        .max_connections(connections)
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

async fn count(conn: &DatabaseConnection, sql: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(
            conn.get_database_backend(),
            sql.to_owned(),
        ))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .expect("a count returns a row");
    row.try_get_by_index::<i64>(0).expect("read the count")
}

/// Fresh `ev_users` and `auth_flow_tokens` tables, users 1 and 2 at
/// `ada-engine@x.com` and `grace-engine@x.com`, and `EloquentUserProvider::<EvUser>` as
/// the default guard's provider.
async fn install(engine: Engine) -> (suprnova::testing::TestContainerGuard, DatabaseConnection) {
    // SAFETY: every test in this file is `#[serial]`; no parallel observer.
    unsafe {
        std::env::set_var("MAIL_FROM", "test-mailer@example.com");
    }
    let conn = connect(engine, 4).await;
    execute(&conn, "DROP TABLE IF EXISTS ev_users").await;
    execute(&conn, "DROP TABLE IF EXISTS auth_flow_tokens").await;
    execute(&conn, engine.create_users()).await;
    conn.execute(&create_auth_flow_tokens_table())
        .await
        .expect("create auth_flow_tokens");
    execute(
        &conn,
        "INSERT INTO ev_users (id, email, password) VALUES \
         (1, 'ada-engine@x.com', 'x'), (2, 'grace-engine@x.com', 'x')",
    )
    .await;

    let guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<EvUser>::new()))
        .expect("register provider");
    suprnova::rate_limit::bootstrap_default().await;
    (guard, conn)
}

async fn user(id: &str) -> EvUser {
    EloquentUserProvider::<EvUser>::new()
        .retrieve_by_id(id)
        .await
        .expect("by id")
        .unwrap_or_else(|| panic!("user {id} exists"))
        .as_any()
        .downcast_ref::<EvUser>()
        .expect("EvUser")
        .clone()
}

/// The token of a verification link mailed to `user`.
async fn link_token(user: &EvUser) -> String {
    let fake = suprnova::mail::Mail::fake();
    EmailVerification::send_link(user, "https://app.test/verify")
        .await
        .expect("send_link");
    let captured = fake.captured();
    let text = captured[0].text.as_deref().expect("text body");
    let link = text
        .lines()
        .find(|line| line.contains("token="))
        .expect("token link");
    link.rsplit("token=")
        .next()
        .expect("token")
        .trim()
        .to_owned()
}

async fn verify_as(user_id: &str, token: &str) -> Result<String, suprnova::FrameworkError> {
    let slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(slot, async {
        suprnova::session::set_auth_user(user_id);
        EmailVerification::verify(token).await
    })
    .await
}

/// Waits until `watcher` sees a statement on `ev_users` wait on a row lock:
/// `verify` is past its read of the address and blocked on the held row.
/// Fails the test if that never happens.
async fn until_verify_waits(watcher: &DatabaseConnection, engine: Engine) {
    for _ in 0..1_000 {
        if count(watcher, engine.waiting_probe()).await > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("verify never waited on the row the second connection held");
}

async fn an_address_changed_by_another_connection_while_verify_runs_is_never_verified(
    engine: Engine,
) {
    let (guard, conn) = install(engine).await;

    // Positive control: with no concurrent change, the link verifies through
    // the same locked transaction on this engine.
    let grace = user("2").await;
    let token = link_token(&grace).await;
    verify_as("2", &token)
        .await
        .expect("an unchanged address verifies");
    assert!(user("2").await.is_email_verified());

    let ada = user("1").await;
    let token = link_token(&ada).await;
    let holder = connect(engine, 1).await;
    let watcher = connect(engine, 1).await;
    let held = holder.begin().await.expect("begin the holder");
    held.execute_unprepared("UPDATE ev_users SET email = 'unproven@x.com' WHERE id = 1")
        .await
        .expect("hold the address change");

    let (outcome, ()) = tokio::join!(verify_as("1", &token), async {
        until_verify_waits(&watcher, engine).await;
        held.commit().await.expect("commit the address change");
    });

    let ada = user("1").await;
    assert_eq!(ada.email, "unproven@x.com", "the change committed");
    assert!(
        !ada.is_email_verified(),
        "a link mailed to ada-engine@x.com must not verify unproven@x.com - verify \
         returned {outcome:?}"
    );
    assert!(outcome.is_err(), "verify reports that nothing was verified");

    holder.close().await.expect("close the holder");
    watcher.close().await.expect("close the watcher");
    execute(&conn, "DROP TABLE IF EXISTS ev_users").await;
    execute(&conn, "DROP TABLE IF EXISTS auth_flow_tokens").await;
    drop(guard);
    conn.close().await.expect("close the test connection");
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_an_address_changed_by_another_connection_while_verify_runs_is_never_verified() {
    let _env = crate::env_lock::lock_env_async().await;
    an_address_changed_by_another_connection_while_verify_runs_is_never_verified(Engine::Postgres)
        .await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_an_address_changed_by_another_connection_while_verify_runs_is_never_verified() {
    let _env = crate::env_lock::lock_env_async().await;
    an_address_changed_by_another_connection_while_verify_runs_is_never_verified(Engine::Mysql)
        .await;
}
