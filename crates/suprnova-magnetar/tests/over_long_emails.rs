//! An email address longer than the default schema can hold never reaches
//! the database.
//!
//! The default schema keeps addresses and email lockout identities in
//! `varchar(255)` on MySQL and MariaDB, which refuse a longer write. No
//! deliverable address is longer than 254 characters (RFC 5321), so
//! Magnetar refuses a longer one at sign-up as a validation error, and a
//! sign-in with one answers exactly as an unknown address does: the same
//! status and body, after the same password work, with no row read or
//! written for it.
//!
//! The stores and services run against a mock database that fails every
//! statement, which proves no statement is sent. The password plugin runs
//! on SQLite in the default suite and on MySQL/MariaDB as a manual, ignored
//! qualification test against `MAGNETAR_MYSQL_TEST_URL`.

#![cfg(all(
    feature = "password",
    feature = "email-verification",
    feature = "password-management",
    feature = "magic-link",
    feature = "passkey",
    feature = "two-factor",
    any(feature = "seaorm-sqlite", feature = "seaorm-mysql")
))]

#[path = "fixtures/password_harness.rs"]
mod harness;
#[cfg(feature = "seaorm-mysql")]
#[path = "fixtures/live_database.rs"]
mod live_database;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use chrono::Utc;
use magnetar::auth::OpaqueFactorGate;
use magnetar::crypto::AeadEncryptor;
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::password::{
    LockoutConfig, LockoutService, PasswordVerifier, StandardPasswordHashDriver,
};
use magnetar::plugin::{PluginContext, PluginRegistry, WireRequest};
use magnetar::plugins::password::{
    PasswordAttempt, PasswordAuthProvider, PasswordAuthService, PasswordPlugin,
    PasswordPluginConfig, RegisterInput,
};
use magnetar::sessions::{OpaqueConfig, OpaqueSessionProvider, SessionMetadata};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DbBackend, MockDatabase, Statement, Transaction,
};
use secrecy::SecretString;

use harness::{
    CountingLimiter, IdentityEncryptor, NoSecondFactor, NoTransport, RecordingMail, Reply,
    TestLinks, fast_hash_config, login_request, register_request, split,
};

const PASSWORD: &str = "orange tabby cat";

/// RFC 5321 limits a forward path to 256 octets, two of them its angle
/// brackets, so no deliverable address is longer than this.
const MAX_EMAIL_LENGTH: usize = 254;

/// An address of exactly `length` characters.
fn email_of_length(length: usize) -> String {
    let domain = "@example.test";
    format!("{}{domain}", "a".repeat(length - domain.len()))
}

fn over_long_email() -> String {
    email_of_length(MAX_EMAIL_LENGTH + 1)
}

fn is_email_validation_error(error: &magnetar::Error) -> bool {
    matches!(error, magnetar::Error::InvalidInput { field, message }
        if field == "email" && message.contains(&MAX_EMAIL_LENGTH.to_string()))
}

/// A connection that fails every statement and records what it was sent.
fn mock_database() -> DatabaseConnection {
    MockDatabase::new(DbBackend::MySql).into_connection()
}

fn statements_sent(db: DatabaseConnection) -> Vec<Transaction> {
    db.into_transaction_log()
}

#[tokio::test]
async fn the_mock_database_fails_a_statement_it_is_sent() {
    let db = mock_database();
    let storage = SeaOrmStorage::<DefaultAuthSchema>::new(db.clone());
    assert!(
        storage
            .find_by_email(&email_of_length(MAX_EMAIL_LENGTH))
            .await
            .is_err(),
        "a lookup the store sends fails, so the other tests prove nothing was sent"
    );
    drop(storage);
    assert_eq!(statements_sent(db).len(), 1);
}

#[tokio::test]
async fn the_user_store_sends_no_statement_for_an_over_long_email() {
    let db = mock_database();
    let storage = SeaOrmStorage::<DefaultAuthSchema>::new(db.clone());
    let email = over_long_email();

    assert_eq!(
        storage
            .find_by_email(&email)
            .await
            .expect("no account holds it"),
        None
    );
    let error = storage
        .create_user(NewUser {
            email: email.clone(),
            password_hash: Some("fixture-hash".to_owned()),
        })
        .await
        .expect_err("an over-long address is refused");
    assert!(is_email_validation_error(&error), "got {error:?}");
    assert!(
        !storage
            .lock_if_unlocked_by_email(&email, Utc::now(), Utc::now())
            .await
            .expect("no account to lock")
    );
    storage
        .set_locked_at_by_email(&email, None)
        .await
        .expect("no account to unlock");

    drop(storage);
    assert_eq!(statements_sent(db), Vec::<Transaction>::new());
}

#[tokio::test]
async fn the_email_lockout_sends_no_statement_for_an_over_long_identity() {
    let db = mock_database();
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let lockout = LockoutService::new(storage.clone(), storage.clone(), LockoutConfig::default());
    let identity = over_long_email();

    let status = lockout.status(&identity).await.expect("status");
    assert!(!status.is_locked);
    assert_eq!(status.failed_attempts, 0);
    let failed = lockout
        .record_failed_attempt(&identity, Some("203.0.113.9"))
        .await
        .expect("a failure for an address no account holds records nothing");
    assert!(!failed.status.is_locked);
    assert!(!failed.locked_event);
    let admission = lockout
        .admit_attempt(&identity, None)
        .await
        .expect("admission");
    assert!(admission.admitted);
    assert!(admission.reservation().is_none());
    lockout
        .finalize_failed_attempt(&identity, &admission)
        .await
        .expect("finalize");
    lockout.reset_attempts(&identity).await.expect("reset");
    assert!(!lockout.unlock_account(&identity).await.expect("unlock"));

    drop((lockout, storage));
    assert_eq!(statements_sent(db), Vec::<Transaction>::new());
}

#[tokio::test]
async fn password_sign_in_and_registration_send_no_statement_for_an_over_long_email() {
    let db = mock_database();
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let verifier = Arc::new(
        PasswordVerifier::new(Arc::new(StandardPasswordHashDriver), fast_hash_config())
            .expect("dummy warmup succeeds"),
    );
    let provider = PasswordAuthService::new(storage.clone(), storage.clone(), verifier);

    let error = provider
        .authenticate_with_outcome(PasswordAttempt {
            email: over_long_email(),
            password: SecretString::from(PASSWORD),
            metadata: SessionMetadata::default(),
        })
        .await
        .expect_err("an over-long address signs no one in");
    assert!(
        matches!(&error, magnetar::Error::InvalidInput { field, message }
            if field == "credentials" && message == "invalid credentials"),
        "the answer is the invalid-credentials answer; got {error:?}"
    );
    provider
        .perform_authentication_work(&over_long_email(), &SecretString::from(PASSWORD))
        .await
        .expect("the equal-cost work runs without a lookup");
    let error = provider
        .register(RegisterInput {
            email: over_long_email(),
            password: SecretString::from(PASSWORD),
        })
        .await
        .expect_err("an over-long address is refused at registration");
    assert!(is_email_validation_error(&error), "got {error:?}");

    drop((provider, storage));
    assert_eq!(statements_sent(db), Vec::<Transaction>::new());
}

/// The password plugin over the default schema, with no second factor.
async fn password_registry(db: &DatabaseConnection) -> PluginRegistry<DefaultAuthSchema> {
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let sessions = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(db.clone())),
        OpaqueConfig::default(),
    ));
    let verifier = Arc::new(
        PasswordVerifier::new(Arc::new(StandardPasswordHashDriver), fast_hash_config())
            .expect("dummy warmup succeeds"),
    );
    let lockout = Arc::new(LockoutService::new(
        storage.clone(),
        storage.clone(),
        LockoutConfig::default(),
    ));
    let provider = Arc::new(PasswordAuthService::new(
        storage.clone(),
        storage.clone(),
        verifier,
    ));
    let gate = Arc::new(OpaqueFactorGate::new(
        storage.clone(),
        Arc::new(NoSecondFactor),
        Arc::new(AeadEncryptor::new([31; 32])),
        sessions.clone(),
    ));
    let context = PluginContext::new(
        storage,
        sessions,
        gate,
        Arc::new(IdentityEncryptor),
        Arc::new(CountingLimiter::default()),
        Arc::new(RecordingMail::default()),
        Arc::new(NoTransport),
        Arc::new(TestLinks),
    );
    PluginRegistry::new(context)
        .register(PasswordPlugin::new(
            provider,
            lockout,
            None,
            None,
            PasswordPluginConfig::default(),
        ))
        .build()
        .await
        .expect("plugin composition is valid")
}

async fn send(registry: &PluginRegistry<DefaultAuthSchema>, request: WireRequest) -> Reply {
    split(
        registry
            .handle(request)
            .await
            .expect("route dispatch succeeds"),
    )
}

async fn row_count(db: &DatabaseConnection, table: &str) -> i64 {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        format!("SELECT COUNT(*) AS rows_counted FROM {table}"),
    ))
    .await
    .expect("count rows")
    .expect("a count row")
    .try_get::<i64>("", "rows_counted")
    .expect("an integer count")
}

/// Over the password plugin: an over-long address signs in as an unknown
/// address does and writes no lockout row; registering one is refused as a
/// validation error and creates no account; an address of exactly 254
/// characters registers and signs in.
async fn verify_over_long_emails(db: &DatabaseConnection) {
    magnetar::default_schema::migrate(db)
        .await
        .expect("create default auth tables");
    let registry = password_registry(db).await;
    let unique = format!("{:016x}", rand::random::<u64>());

    let before = row_count(db, "auth_lockouts").await;
    let unknown = send(
        &registry,
        login_request(&format!("nobody-{unique}@example.test"), PASSWORD),
    )
    .await;
    assert_eq!(unknown.status, 401, "an unknown address is refused");
    let lockouts = row_count(db, "auth_lockouts").await;
    assert!(
        lockouts > before,
        "a failure for an unknown address is counted"
    );
    // One character past the bound, and far past the column, where MySQL
    // and MariaDB would refuse the lockout row.
    for length in [MAX_EMAIL_LENGTH + 1, 300] {
        let email = email_of_length(length);
        let over_long = send(&registry, login_request(&email, PASSWORD)).await;
        assert_eq!(over_long.status, unknown.status, "{length} characters");
        assert_eq!(over_long.body, unknown.body, "{length} characters");
        assert!(over_long.grant.is_none());
        assert_eq!(
            row_count(db, "auth_lockouts").await,
            lockouts,
            "nothing is written for a {length}-character address"
        );
    }

    let users = row_count(db, "app_users").await;
    for length in [MAX_EMAIL_LENGTH + 1, 300] {
        let refused = send(
            &registry,
            register_request(&email_of_length(length), PASSWORD),
        )
        .await;
        assert_eq!(
            refused.status, 400,
            "registration refuses a {length}-character address"
        );
        let message = refused
            .body
            .as_ref()
            .and_then(|body| body["message"].as_str())
            .unwrap_or_default()
            .to_owned();
        assert!(
            message.contains(&MAX_EMAIL_LENGTH.to_string()),
            "the refusal names the limit; got {message:?}"
        );
        assert_eq!(row_count(db, "app_users").await, users);
    }

    let longest = format!(
        "{unique}{}",
        email_of_length(MAX_EMAIL_LENGTH - unique.len())
    );
    assert_eq!(longest.chars().count(), MAX_EMAIL_LENGTH);
    let registered = send(&registry, register_request(&longest, PASSWORD)).await;
    assert_eq!(registered.status, 200, "a 254-character address registers");
    assert_eq!(row_count(db, "app_users").await, users + 1);
    let signed_in = send(&registry, login_request(&longest, PASSWORD)).await;
    assert_eq!(signed_in.status, 200, "and signs in");
    assert!(signed_in.grant.is_some());
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_over_long_emails_never_reach_the_database() {
    let db = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    verify_over_long_emails(&db).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_over_long_emails_never_reach_the_database() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let live = live_database::LiveDatabase::create(&url, "my_emails").await;
    let db = live.connect().await;
    verify_over_long_emails(&db).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}
