//! HRD-008: the job that retries a password reset's revocations.
//!
//! `RevokeUserCredentials` revokes a user's sessions and remember-me tokens.
//! It fails while either store refuses, so the queue runs it again, and it
//! succeeds once both stores confirm. These tests drive it with a session
//! store that refuses until the test heals it, and with a `remember_tokens`
//! table that the test creates or leaves out.
//!
//! The reset tests below complete a reset through the verified-provider
//! path and read what it queued. They hold the environment lock, as every
//! test in this binary that completes a reset or fakes mail does: the reset
//! sends a `PasswordChangedMail` when `MAIL_FROM` is set, and the mail fake
//! is process-wide. The user has an id no other test in this binary uses,
//! because a reset elsewhere in the process can push into the queue fake
//! while one of these holds it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use suprnova::auth::AuthConfig;
use suprnova::auth_flows::token_store::{TokenStore, create_auth_flow_tokens_table};
use suprnova::auth_flows::{PasswordReset, RevocationRetry, RevokeUserCredentials, TokenPurpose};
use suprnova::container::testing::TestContainer;
use suprnova::queue::testing::{pushed, reserved};
use suprnova::queue::worker::WorkerConfig;
use suprnova::queue::{Queue, QueueDriver};
use suprnova::session::{SessionData, SessionStore};
use suprnova::testing::{TestClock, TestDatabase};
use suprnova::{
    Auth, AuthManager, EloquentUserProvider, FrameworkError, Job, WorkerControls, async_trait,
    run_worker_with_controls,
};

use crate::password_reset_provider::TestUser;

/// A session store that holds, per user, how many sessions the default guard
/// is signed in to, and refuses every revocation while `refusing` is set.
#[derive(Default)]
struct RefusingSessionStore {
    sessions: Mutex<HashMap<String, u64>>,
    refusing: AtomicBool,
    /// What each revocation the store carried out returned, in order.
    revocations: Mutex<Vec<u64>>,
}

impl RefusingSessionStore {
    fn holding(users: &[(&str, u64)]) -> Arc<Self> {
        let store = Self::default();
        store.sessions.lock().expect("sessions").extend(
            users
                .iter()
                .map(|(user_id, count)| ((*user_id).to_owned(), *count)),
        );
        Arc::new(store)
    }

    fn refuse(&self, refusing: bool) {
        self.refusing.store(refusing, Ordering::SeqCst);
    }

    fn sessions_of(&self, user_id: &str) -> u64 {
        self.sessions
            .lock()
            .expect("sessions")
            .get(user_id)
            .copied()
            .unwrap_or(0)
    }

    fn revocations(&self) -> Vec<u64> {
        self.revocations.lock().expect("revocations").clone()
    }
}

#[async_trait]
impl SessionStore for RefusingSessionStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }

    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        if self.refusing.load(Ordering::SeqCst) {
            return Err(FrameworkError::database("the session store is unavailable"));
        }
        let revoked = self
            .sessions
            .lock()
            .expect("sessions")
            .remove(user_id)
            .unwrap_or(0);
        self.revocations.lock().expect("revocations").push(revoked);
        Ok(revoked)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// The shape of the `remember_tokens` table the framework reads.
const CREATE_REMEMBER_TOKENS: &str = "CREATE TABLE remember_tokens (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, \
    user_id TEXT NOT NULL, \
    selector TEXT NOT NULL UNIQUE, \
    token_hash TEXT NOT NULL, \
    expires_at DATETIME NOT NULL, \
    created_at DATETIME NOT NULL, \
    last_used_at DATETIME)";

/// Create `remember_tokens` and give each of `users` one token.
async fn create_remember_tokens(database: &TestDatabase, users: &[&str]) {
    database
        .execute_unprepared(CREATE_REMEMBER_TOKENS)
        .await
        .expect("create remember_tokens");
    for user_id in users {
        database
            .execute_unprepared(&format!(
                "INSERT INTO remember_tokens \
                 (user_id, selector, token_hash, expires_at, created_at) VALUES \
                 ('{user_id}', 'selector-{user_id}', 'hash', \
                  '2099-01-01 00:00:00', '2026-01-01 00:00:00')"
            ))
            .await
            .expect("insert a remember token");
    }
}

async fn remember_tokens_of(database: &TestDatabase, user_id: &str) -> i64 {
    database
        .fetch_one(
            "SELECT COUNT(*) AS n FROM remember_tokens WHERE user_id = ?",
            vec![user_id.into()],
        )
        .await
        .expect("count remember tokens")
        .try_get::<i64>("", "n")
        .expect("read the count")
}

/// Make `store` the session store the revocation resolves. The binding is
/// on this test's thread, under the container `TestDatabase` installed.
fn bind_session_store(store: &Arc<RefusingSessionStore>) {
    let store: Arc<dyn SessionStore> = store.clone();
    TestContainer::bind::<dyn SessionStore>(store);
}

/// Run a worker over `driver` until it finds nothing due.
async fn drain(driver: Arc<dyn QueueDriver>) {
    let status = run_worker_with_controls(
        driver,
        WorkerConfig {
            poll_interval: Duration::from_millis(5),
            ..WorkerConfig::default()
        },
        WorkerControls {
            stop_when_empty: true,
            ..WorkerControls::default()
        },
        CancellationToken::new(),
    )
    .await
    .expect("the worker runs");
    assert_eq!(status, 0, "the worker stops normally");
}

/// The falsifier's second half: the job does not report done while the
/// session store still holds the user's sessions. A worker runs it from
/// the queue; each refused try puts it back after its backoff, and the try
/// after the store recovers completes it. The job is not registered here:
/// the framework registers it with the worker.
#[tokio::test]
async fn hardening_the_revocation_retry_runs_again_while_a_store_refuses_and_stops_once_both_confirm()
 {
    let database = TestDatabase::sqlite_memory()
        .await
        .expect("sqlite database");
    create_remember_tokens(&database, &["7", "8"]).await;
    let store = RefusingSessionStore::holding(&[("7", 2), ("8", 1)]);
    store.refuse(true);
    bind_session_store(&store);
    // The worker runs on this thread, so it reads this clock.
    let clock = TestClock::freeze();
    let fake = Queue::fake();

    Queue::push(RevokeUserCredentials::new("7"))
        .await
        .expect("queue the retry");

    drain(fake.driver()).await;
    assert_eq!(reserved::<RevokeUserCredentials>().len(), 1, "one try ran");
    assert_eq!(
        store.sessions_of("7"),
        2,
        "the store refused, so the sessions remain"
    );
    assert_eq!(
        remember_tokens_of(&database, "7").await,
        0,
        "the remember-me tokens are revoked on the same try"
    );

    drain(fake.driver()).await;
    assert_eq!(
        reserved::<RevokeUserCredentials>().len(),
        1,
        "the retry waits for its backoff"
    );

    clock.advance(chrono::Duration::hours(2));
    drain(fake.driver()).await;
    assert_eq!(
        reserved::<RevokeUserCredentials>().len(),
        2,
        "the refused try was retried"
    );
    assert_eq!(store.sessions_of("7"), 2, "the store still refuses");

    store.refuse(false);
    clock.advance(chrono::Duration::hours(2));
    drain(fake.driver()).await;
    let tries = Queue::reserved_jobs(None)
        .await
        .expect("the reservations the fake recorded");
    assert_eq!(
        tries.iter().map(|job| job.attempts).collect::<Vec<_>>(),
        vec![0, 1, 2],
        "each retry carries the tries spent before it"
    );
    assert_eq!(store.sessions_of("7"), 0, "the sessions are revoked");
    assert_eq!(store.sessions_of("8"), 1, "another user's sessions stay");
    assert_eq!(
        remember_tokens_of(&database, "8").await,
        1,
        "another user's remember-me token stays"
    );

    clock.advance(chrono::Duration::hours(2));
    drain(fake.driver()).await;
    assert_eq!(
        reserved::<RevokeUserCredentials>().len(),
        3,
        "the job completed and was not tried again"
    );
}

/// The job is idempotent: a run after the credentials are gone succeeds and
/// revokes nothing.
#[tokio::test]
async fn hardening_the_revocation_retry_is_a_no_op_once_the_credentials_are_gone() {
    let database = TestDatabase::sqlite_memory()
        .await
        .expect("sqlite database");
    create_remember_tokens(&database, &["7", "8"]).await;
    let store = RefusingSessionStore::holding(&[("7", 2), ("8", 1)]);
    bind_session_store(&store);

    RevokeUserCredentials::new("7")
        .handle()
        .await
        .expect("the first run revokes both");
    assert_eq!(store.sessions_of("7"), 0);
    assert_eq!(remember_tokens_of(&database, "7").await, 0);

    RevokeUserCredentials::new("7")
        .handle()
        .await
        .expect("a run after the credentials are gone succeeds");
    assert_eq!(
        store.revocations(),
        vec![2, 0],
        "the second run found no session to revoke"
    );
    assert_eq!(store.sessions_of("8"), 1, "another user's sessions stay");
    assert_eq!(remember_tokens_of(&database, "8").await, 1);
}

/// Either store can refuse: while the remember-me tokens cannot be revoked
/// the job fails, although the sessions were revoked on the same run.
#[tokio::test]
async fn hardening_the_revocation_retry_fails_while_the_remember_token_store_refuses() {
    // No `remember_tokens` table yet: every revocation of a token fails.
    let database = TestDatabase::sqlite_memory()
        .await
        .expect("sqlite database");
    let store = RefusingSessionStore::holding(&[("7", 1)]);
    bind_session_store(&store);

    let error = RevokeUserCredentials::new("7")
        .handle()
        .await
        .expect_err("the job fails while the remember-me tokens cannot be revoked");
    assert!(
        error.to_string().contains("remember"),
        "the error names the store that refused: {error}"
    );
    assert_eq!(
        store.sessions_of("7"),
        0,
        "the session store did not wait for the other store"
    );

    create_remember_tokens(&database, &["7"]).await;
    RevokeUserCredentials::new("7")
        .handle()
        .await
        .expect("the job succeeds once both stores confirm");
    assert_eq!(remember_tokens_of(&database, "7").await, 0);
}

/// The id of the user the reset tests reset.
const RESET_USER: &str = "4242";

/// A database with a verified user `RESET_USER`, the token and remember-me
/// tables, and the provider a reset rotates the password through.
async fn reset_database() -> TestDatabase {
    let database = TestDatabase::sqlite_memory()
        .await
        .expect("sqlite database");
    database
        .execute_unprepared(
            "CREATE TABLE users (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                email TEXT NOT NULL, \
                password TEXT NOT NULL, \
                email_verified_at TEXT\
             )",
        )
        .await
        .expect("create the users table");
    database
        .execute_unprepared(&format!(
            "INSERT INTO users (id, email, password, email_verified_at) VALUES \
             ({RESET_USER}, 'retry@example.test', 'hash-before-reset', CURRENT_TIMESTAMP)"
        ))
        .await
        .expect("insert the user");
    {
        use sea_orm::ConnectionTrait;
        database
            .conn()
            .execute(&create_auth_flow_tokens_table())
            .await
            .expect("create the auth-flow token table");
    }
    create_remember_tokens(&database, &[RESET_USER]).await;
    TestContainer::singleton(AuthManager::new(AuthConfig::default()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<TestUser>::new()))
        .expect("register the provider");
    database
}

async fn reset_token() -> String {
    TokenStore::issue(
        RESET_USER,
        TokenPurpose::PasswordReset,
        TokenPurpose::PasswordReset.default_ttl(),
    )
    .await
    .expect("issue a reset token")
}

/// The revocation retries the queue fake holds for `RESET_USER`.
fn retries_queued() -> usize {
    pushed::<RevokeUserCredentials>()
        .iter()
        .filter(|job| job.user_id == RESET_USER)
        .count()
}

/// The falsifier's first half: the session store refuses, so completing the
/// reset queues one retry for the user before it returns. `complete`, which
/// hides the outcome from its caller, queues it the same way.
#[tokio::test]
async fn hardening_a_reset_whose_revocation_fails_queues_one_revocation_retry() {
    let _env = crate::env_lock::lock_env_async().await;
    let _database = reset_database().await;
    let store = RefusingSessionStore::holding(&[(RESET_USER, 1)]);
    store.refuse(true);
    bind_session_store(&store);
    let _queue = Queue::fake();

    let outcome = PasswordReset::complete_with_outcome(&reset_token().await, "new-password")
        .await
        .expect("the password changes although a revocation failed");
    assert_eq!(outcome.user_id, RESET_USER);
    assert!(outcome.sessions_revoked.is_err(), "the store refused");
    assert_eq!(
        outcome.remember_tokens_revoked.as_ref().ok(),
        Some(&1),
        "the remember-me token was revoked"
    );
    assert_eq!(retries_queued(), 1, "one revocation retry is queued");
    assert!(
        matches!(outcome.revocation_retry, RevocationRetry::Queued),
        "the outcome reports the queued retry: {:?}",
        outcome.revocation_retry
    );

    PasswordReset::complete(&reset_token().await, "another-password")
        .await
        .expect("complete rotates the password");
    assert_eq!(retries_queued(), 2, "complete queues the retry too");
}

/// With both stores healthy nothing is retried, so nothing is queued.
#[tokio::test]
async fn hardening_a_reset_whose_revocations_succeed_queues_no_retry() {
    let _env = crate::env_lock::lock_env_async().await;
    let _database = reset_database().await;
    let store = RefusingSessionStore::holding(&[(RESET_USER, 1)]);
    bind_session_store(&store);
    let _queue = Queue::fake();

    let outcome = PasswordReset::complete_with_outcome(&reset_token().await, "new-password")
        .await
        .expect("the reset completes");
    assert_eq!(outcome.sessions_revoked.as_ref().ok(), Some(&1));
    assert_eq!(outcome.remember_tokens_revoked.as_ref().ok(), Some(&1));
    assert_eq!(retries_queued(), 0, "a healthy reset queued a retry");
    assert!(
        matches!(outcome.revocation_retry, RevocationRetry::NotNeeded),
        "{:?}",
        outcome.revocation_retry
    );
}

/// A retry that cannot be queued is reported in the outcome, and the reset
/// still returns the outcome, because the password has already changed.
#[tokio::test]
async fn hardening_a_reset_reports_a_retry_it_could_not_queue() {
    let _env = crate::env_lock::lock_env_async().await;
    let _database = reset_database().await;
    let store = RefusingSessionStore::holding(&[(RESET_USER, 1)]);
    store.refuse(true);
    bind_session_store(&store);
    // The retry takes the real path, and with no driver installed the push
    // fails. No other test in this binary installs a queue driver.
    let _queue = Queue::fake_except(&[RevokeUserCredentials::job_name()]);
    suprnova::queue::testing::forget_driver();

    let outcome = PasswordReset::complete_with_outcome(&reset_token().await, "new-password")
        .await
        .expect("the password changes although the retry could not be queued");
    assert!(outcome.sessions_revoked.is_err(), "the store refused");
    let RevocationRetry::PushFailed(error) = &outcome.revocation_retry else {
        panic!(
            "expected the failed push, got {:?}",
            outcome.revocation_retry
        );
    };
    assert!(
        error.to_string().contains("queue driver not initialized"),
        "the outcome carries the push's error: {error}"
    );
    assert_eq!(retries_queued(), 0, "the excepted retry was not recorded");
}
