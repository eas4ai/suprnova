//! Values longer than the default schema's columns hold, from sign-in,
//! sign-up and request headers.
//!
//! The default schema keeps addresses, lockout identities and lockout
//! reasons in `varchar(255)` on MySQL and MariaDB, which refuse a longer
//! write. No deliverable address is longer than 254 characters (RFC 5321),
//! so Magnetar refuses a longer one at sign-up as a validation error, and a
//! sign-in with one answers exactly as an unknown address does: the same
//! status and body, after the same password work, without a user lookup.
//! A lockout identity that long is counted under a fixed-length digest, a
//! lockout reason keeps only a client address that parses as one, and a
//! session keeps at most 1,024 characters of its user agent.
//!
//! The user store and password service run against a mock database that
//! fails every statement, which proves no statement is sent. The rest runs
//! on SQLite in the default suite and on MySQL/MariaDB as manual, ignored
//! qualification tests against `MAGNETAR_MYSQL_TEST_URL`.

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
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::default_schema::{DefaultAuthSchema, sessions};
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
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, MockDatabase,
    QueryFilter, Statement, Transaction,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

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
    assert!(
        row_count(db, "auth_lockouts").await > before,
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
            identity_rows(db, &email).await,
            0,
            "the {length}-character address itself is never stored"
        );
        assert!(
            identity_rows(db, &digest_key(&email)).await > 0,
            "the failure is counted under the address's digest"
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

/// The key a lockout identity longer than 254 characters is counted under.
fn digest_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

/// How many lockout rows carry `identity`.
async fn identity_rows(db: &DatabaseConnection, identity: &str) -> i64 {
    let backend = db.get_database_backend();
    let placeholder = if backend == DbBackend::Postgres {
        "$1"
    } else {
        "?"
    };
    db.query_one_raw(Statement::from_sql_and_values(
        backend,
        format!(
            "SELECT COUNT(*) AS rows_counted FROM auth_lockouts WHERE identity = {placeholder}"
        ),
        [identity.to_owned().into()],
    ))
    .await
    .expect("count lockout rows")
    .expect("a count row")
    .try_get::<i64>("", "rows_counted")
    .expect("an integer count")
}

/// The reasons of `identity`'s attempt rows, leaving out the store's own
/// serialization row.
async fn attempt_reasons(db: &DatabaseConnection, identity: &str) -> Vec<Option<String>> {
    magnetar::default_schema::lockouts::Entity::find()
        .filter(magnetar::default_schema::lockouts::Column::Identity.eq(identity))
        .all(db)
        .await
        .expect("read lockout rows")
        .into_iter()
        .map(|row| row.reason)
        .filter(|reason| {
            !reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("__magnetar_lockout_"))
        })
        .collect()
}

fn unique_label() -> String {
    format!("{:016x}", rand::random::<u64>())
}

/// A lockout identity of 300 characters is counted and locked under its
/// digest; the identity itself is never stored.
async fn verify_over_long_identities_are_counted_under_their_digest(db: &DatabaseConnection) {
    magnetar::default_schema::migrate(db)
        .await
        .expect("create default auth tables");
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let lockout = LockoutService::new(
        storage.clone(),
        storage,
        LockoutConfig {
            max_failed_attempts: 3,
            ..LockoutConfig::default()
        },
    );
    let unique = unique_label();
    let identity = format!("{unique}{}", email_of_length(300 - unique.len()));

    for _ in 0..3 {
        lockout
            .record_failed_attempt(&identity, None)
            .await
            .expect("a failure for an over-long identity is counted");
    }
    let status = lockout.status(&identity).await.expect("status");
    assert!(status.is_locked, "three failures lock it");
    assert_eq!(status.failed_attempts, 3);
    assert_eq!(status.identity, identity, "the status names the identity");
    assert_eq!(identity_rows(db, &identity).await, 0);
    assert!(identity_rows(db, &digest_key(&identity)).await >= 3);

    // A reserved attempt counts under the same key.
    let reserved = format!("r{unique}{}", email_of_length(300 - unique.len() - 1));
    let admission = lockout
        .admit_attempt(&reserved, None)
        .await
        .expect("admission");
    assert!(admission.admitted);
    assert!(admission.reservation().is_some(), "the attempt is reserved");
    lockout
        .finalize_failed_attempt(&reserved, &admission)
        .await
        .expect("finalize");
    assert_eq!(
        lockout
            .status(&reserved)
            .await
            .expect("status")
            .failed_attempts,
        1
    );

    lockout.reset_attempts(&identity).await.expect("reset");
    let reset = lockout.status(&identity).await.expect("status");
    assert_eq!(reset.failed_attempts, 0);
    assert!(!reset.is_locked);
}

/// A lockout reason keeps the client address only when it parses as one.
/// A longer value, as a host that passes a client's header through would
/// give, neither fails the attempt nor reaches the column.
async fn verify_lockout_reasons_keep_only_client_addresses(db: &DatabaseConnection) {
    magnetar::default_schema::migrate(db)
        .await
        .expect("create default auth tables");
    let storage = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    let lockout = LockoutService::new(storage.clone(), storage, LockoutConfig::default());
    let unique = unique_label();
    let over_long = "x".repeat(300);
    let cases = [
        ("203.0.113.9", Some("203.0.113.9")),
        ("2001:db8::7", Some("2001:db8::7")),
        (over_long.as_str(), None),
        ("not an address", None),
    ];
    for (index, (context, stored)) in cases.into_iter().enumerate() {
        let identity = format!("reason-{index}-{unique}@example.test");
        lockout
            .record_failed_attempt(&identity, Some(context))
            .await
            .expect("the attempt is counted whatever the context");
        assert_eq!(
            attempt_reasons(db, &identity).await,
            vec![stored.map(str::to_owned)],
            "context {index}"
        );
    }

    // The password plugin passes the client address header the same way.
    let registry = password_registry(db).await;
    let identity = format!("header-{unique}@example.test");
    let mut request = login_request(&identity, PASSWORD);
    request
        .headers
        .insert("x-client-ip".to_owned(), over_long.clone());
    let reply = send(&registry, request).await;
    assert_eq!(reply.status, 401, "the sign-in is refused, not failed");
    assert_eq!(attempt_reasons(db, &identity).await, vec![None]);
}

/// A session keeps at most 1,024 characters of its user agent, cut on a
/// character boundary, and no client address that is not one.
async fn verify_session_user_agents_are_bounded(db: &DatabaseConnection) {
    magnetar::default_schema::migrate(db)
        .await
        .expect("create default auth tables");
    let registry = password_registry(db).await;
    let email = format!("agent-{}@example.test", unique_label());
    let registered = send(&registry, register_request(&email, PASSWORD)).await;
    assert_eq!(registered.status, 200);

    let agent = format!("{}{}", "u".repeat(1020), "\u{e9}".repeat(68_980));
    assert_eq!(agent.chars().count(), 70_000);
    let mut request = login_request(&email, PASSWORD);
    request
        .headers
        .insert("user-agent".to_owned(), agent.clone());
    request
        .headers
        .insert("x-client-ip".to_owned(), "not an address".to_owned());
    let reply = send(&registry, request).await;
    assert_eq!(reply.status, 200, "the sign-in succeeds");
    let grant = reply.grant.expect("a session");
    let session = sessions::Entity::find_by_id(grant.session_id().to_owned())
        .one(db)
        .await
        .expect("read the session")
        .expect("the session is stored");
    let expected = agent.chars().take(1024).collect::<String>();
    assert_eq!(session.user_agent.as_deref(), Some(expected.as_str()));
    assert_eq!(
        grant.metadata().user_agent.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(session.ip_address, None);
}

#[cfg(feature = "seaorm-sqlite")]
async fn sqlite_memory() -> DatabaseConnection {
    sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite")
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_over_long_lockout_identities_are_counted_under_their_digest() {
    verify_over_long_identities_are_counted_under_their_digest(&sqlite_memory().await).await;
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_lockout_reasons_keep_only_client_addresses() {
    verify_lockout_reasons_keep_only_client_addresses(&sqlite_memory().await).await;
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_session_user_agents_are_bounded() {
    verify_session_user_agents_are_bounded(&sqlite_memory().await).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_over_long_lockout_identities_are_counted_under_their_digest() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let live = live_database::LiveDatabase::create(&url, "my_digest").await;
    let db = live.connect().await;
    verify_over_long_identities_are_counted_under_their_digest(&db).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_lockout_reasons_keep_only_client_addresses() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let live = live_database::LiveDatabase::create(&url, "my_reasons").await;
    let db = live.connect().await;
    verify_lockout_reasons_keep_only_client_addresses(&db).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_session_user_agents_are_bounded() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let live = live_database::LiveDatabase::create(&url, "my_agents").await;
    let db = live.connect().await;
    verify_session_user_agents_are_bounded(&db).await;
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

/// A legacy account whose address passes 254 characters stops the import
/// with the importer's per-row error, before any row is written.
#[cfg(all(feature = "migration", feature = "seaorm-sqlite"))]
#[tokio::test]
async fn sqlite_migration_import_refuses_an_over_long_address() {
    use magnetar::default_migration::DefaultMigrationBindings;
    use magnetar::migration::{MigrationEngine, MigrationRunner, ShapeConfirmation, SourceShape};

    let db = sqlite_memory().await;
    db.execute_unprepared(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL, \
         password TEXT NOT NULL, remember_token TEXT, email_verified_at TEXT, created_at TEXT, \
         updated_at TEXT)",
    )
    .await
    .expect("create the legacy users table");
    db.execute_unprepared(&format!(
        "INSERT INTO users (id, name, email, password) VALUES \
         (1, 'Short', 'short@example.test', 'hash-short'), \
         (2, 'Long', '{}', 'hash-long')",
        over_long_email()
    ))
    .await
    .expect("seed the legacy users");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create the default tables beside the source");
    let runner = MigrationEngine::new(
        db.clone(),
        DefaultMigrationBindings::new(db.clone()).sharing_source_database(),
    );
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaWeb,
            operator_selected: SourceShape::SuprnovaWeb,
        })
        .await
        .expect("plan the import");
    let error = runner
        .apply(&plan)
        .await
        .expect_err("an over-long address stops the import");
    assert!(
        matches!(&error, magnetar::Error::InvalidInput { field, message }
            if field == "email" && message.contains("254") && message.contains("source user 2")
                && !message.contains("aaaa")),
        "the per-row error names the row, not the address: {error:?}"
    );
    assert_eq!(row_count(&db, "app_users").await, 0, "no row is imported");
}
