//! `EmailVerificationService::verify_presented` and the address binding of
//! the tokens it issues.
//!
//! A verification link proves the address it was mailed to. The service
//! binds each token it issues to that address, refuses a token when the
//! account's address is no longer that address, and stamps the address
//! only while the account still holds it. A store that cannot read a
//! token's state, keep its address or stamp on the address makes the
//! service refuse: it never verifies without the check.

#![cfg(all(feature = "email-verification", feature = "seaorm-sqlite"))]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use magnetar::Error;
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::plugins::email_verification::{
    EMAIL_VERIFICATION_PURPOSE, EmailVerificationOutcome, EmailVerificationService,
};
use magnetar::storage::{
    AuthTransaction, ConsumedToken, CredentialActor, IssueToken, IssuedToken, NewUser,
    PresentedToken, SeaOrmStorage, TokenStore, UserRecord, UserStore, address_digest,
};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use secrecy::ExposeSecret;

/// A fresh SQLite database with Magnetar's default tables.
async fn default_database() -> DatabaseConnection {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create the default tables");
    db
}

/// The SQL store over the default tables.
fn default_store(db: &DatabaseConnection) -> SeaOrmStorage<DefaultAuthSchema> {
    SeaOrmStorage::<DefaultAuthSchema>::new(db.clone())
}

/// A user with no password at `email`.
async fn user(store: &SeaOrmStorage<DefaultAuthSchema>, email: &str) -> UserRecord {
    store
        .create_user(NewUser {
            email: email.to_owned(),
            password_hash: None,
        })
        .await
        .expect("create the user")
}

/// The service over one store for users and tokens, mailing nothing.
fn service<U, T>(users: U, tokens: T) -> EmailVerificationService
where
    U: UserStore + 'static,
    T: TokenStore + 'static,
{
    EmailVerificationService::without_delivery(Arc::new(users), Arc::new(tokens))
}

/// One minute of a token of `purpose` for user `user_id`.
fn token_input(user_id: &str, purpose: &str) -> IssueToken {
    IssueToken {
        user_id: user_id.to_owned(),
        purpose: purpose.to_owned(),
        ttl: Duration::from_secs(60),
    }
}

/// Whether `user_id`'s address reads as verified.
async fn verified(store: &SeaOrmStorage<DefaultAuthSchema>, user_id: &str) -> bool {
    store
        .find_by_id(user_id)
        .await
        .expect("read the user")
        .expect("the user")
        .email_verified_at
        .is_some()
}

/// Whether `token` is still a live verification token.
async fn live(store: &SeaOrmStorage<DefaultAuthSchema>, token: &str) -> bool {
    store
        .check(
            PresentedToken::new(token.to_owned()),
            EMAIL_VERIFICATION_PURPOSE,
        )
        .await
        .expect("check the token")
}

/// A token store that keeps tokens but cannot report one's state or keep
/// its address: every other method forwards to the SQL store, and
/// `inspect` and `issue_bound` keep their defaults.
struct NoTokenState(SeaOrmStorage<DefaultAuthSchema>);

#[async_trait]
impl TokenStore for NoTokenState {
    async fn issue(&self, input: IssueToken) -> magnetar::Result<IssuedToken> {
        self.0.issue(input).await
    }

    async fn check(&self, token: PresentedToken, purpose: &str) -> magnetar::Result<bool> {
        self.0.check(token, purpose).await
    }

    async fn consume(
        &self,
        token: PresentedToken,
        purpose: &str,
    ) -> magnetar::Result<ConsumedToken> {
        self.0.consume(token, purpose).await
    }

    async fn consume_in(
        &self,
        tx: &mut AuthTransaction<'_>,
        token: PresentedToken,
        purpose: &str,
    ) -> magnetar::Result<ConsumedToken> {
        self.0.consume_in(tx, token, purpose).await
    }
}

/// A user store that cannot stamp an address on condition that it is
/// still the account's: every other method forwards to the SQL store, and
/// `mark_email_verified_for` keeps its default.
struct NoConditionalStamp(SeaOrmStorage<DefaultAuthSchema>);

#[async_trait]
impl UserStore for NoConditionalStamp {
    async fn find_by_email(&self, email: &str) -> magnetar::Result<Option<UserRecord>> {
        self.0.find_by_email(email).await
    }

    async fn find_by_id(&self, user_id: &str) -> magnetar::Result<Option<UserRecord>> {
        self.0.find_by_id(user_id).await
    }

    async fn create_user(&self, input: NewUser) -> magnetar::Result<UserRecord> {
        self.0.create_user(input).await
    }

    async fn set_password_hash(
        &self,
        actor: &CredentialActor,
        password_hash: &str,
    ) -> magnetar::Result<()> {
        self.0.set_password_hash(actor, password_hash).await
    }

    async fn mark_email_verified(&self, user_id: &str, at: DateTime<Utc>) -> magnetar::Result<()> {
        self.0.mark_email_verified(user_id, at).await
    }

    async fn lock_if_unlocked_by_email(
        &self,
        email: &str,
        locked_at: DateTime<Utc>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        self.0
            .lock_if_unlocked_by_email(email, locked_at, window_start)
            .await
    }

    async fn set_locked_at_by_email(
        &self,
        email: &str,
        locked_at: Option<DateTime<Utc>>,
    ) -> magnetar::Result<()> {
        self.0.set_locked_at_by_email(email, locked_at).await
    }
}

#[tokio::test]
async fn verify_presented_refuses_on_a_store_that_cannot_report_a_token() {
    let db = default_database().await;
    let store = default_store(&db);
    let ada = user(&store, "ada@example.test").await;
    let token = service(store.clone(), store.clone())
        .issue(&ada.user_id, &ada.email)
        .await
        .expect("issue a bound token");
    let token = token.expose_secret().to_owned();
    let blind = service(store.clone(), NoTokenState(store.clone()));

    let refused = blind.verify_presented(&ada.user_id, &token).await;
    assert!(
        matches!(refused, Err(Error::DependencyUnavailable { .. })),
        "{refused:?}"
    );
    assert!(!verified(&store, &ada.user_id).await, "nothing is stamped");
    assert!(live(&store, &token).await, "the token is not spent");

    let unbound = blind.issue(&ada.user_id, &ada.email).await;
    assert!(
        matches!(unbound, Err(Error::DependencyUnavailable { .. })),
        "a store that cannot keep the address issues nothing: {unbound:?}"
    );
}

#[tokio::test]
async fn verify_presented_refuses_on_a_user_store_that_cannot_stamp_on_the_address() {
    let db = default_database().await;
    let store = default_store(&db);
    let ada = user(&store, "ada@example.test").await;
    let token = service(store.clone(), store.clone())
        .issue(&ada.user_id, &ada.email)
        .await
        .expect("issue a bound token");
    let refused = service(NoConditionalStamp(store.clone()), store.clone())
        .verify_presented(&ada.user_id, token.expose_secret())
        .await;
    assert!(
        matches!(refused, Err(Error::DependencyUnavailable { .. })),
        "{refused:?}"
    );
    assert!(!verified(&store, &ada.user_id).await, "nothing is stamped");
}

#[tokio::test]
async fn verify_presented_refuses_a_token_after_the_address_changed() {
    let db = default_database().await;
    let store = default_store(&db);
    let ada = user(&store, "ada@example.test").await;
    let verification = service(store.clone(), store.clone());
    let token = verification
        .issue(&ada.user_id, &ada.email)
        .await
        .expect("issue a bound token");
    let token = token.expose_secret().to_owned();
    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!(
            "UPDATE app_users SET email = 'bob@example.test' WHERE id = {}",
            ada.user_id
        ),
    ))
    .await
    .expect("change the address");

    assert_eq!(
        verification
            .verify_presented(&ada.user_id, &token)
            .await
            .expect("present the token"),
        EmailVerificationOutcome::AddressChanged
    );
    assert!(!verified(&store, &ada.user_id).await, "nothing is stamped");

    // The link proves the address it was mailed to, so it verifies again
    // once the account holds that address.
    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        format!(
            "UPDATE app_users SET email = 'ada@example.test' WHERE id = {}",
            ada.user_id
        ),
    ))
    .await
    .expect("restore the address");
    assert_eq!(
        verification
            .verify_presented(&ada.user_id, &token)
            .await
            .expect("present the token"),
        EmailVerificationOutcome::Verified
    );
    assert!(verified(&store, &ada.user_id).await);
}

#[tokio::test]
async fn verify_presented_refuses_a_token_bound_to_no_address() {
    let db = default_database().await;
    let store = default_store(&db);
    let ada = user(&store, "ada@example.test").await;
    let unbound = store
        .issue(token_input(&ada.user_id, EMAIL_VERIFICATION_PURPOSE))
        .await
        .expect("issue an unbound token");
    let unbound = unbound.plaintext.expose_secret().to_owned();
    let verification = service(store.clone(), store.clone());
    assert_eq!(
        verification
            .verify_presented(&ada.user_id, &unbound)
            .await
            .expect("present the token"),
        EmailVerificationOutcome::Unknown
    );
    assert!(!verified(&store, &ada.user_id).await);

    // Magnetar's own route, `verify`, keeps accepting such a token.
    verification
        .verify(&ada.user_id, &unbound)
        .await
        .expect("the route's verify");
    assert!(verified(&store, &ada.user_id).await);
}

#[tokio::test]
async fn the_default_schema_binds_a_token_to_the_lowercased_address() {
    let db = default_database().await;
    let store = default_store(&db);
    let issued = store
        .issue_bound(
            token_input("7", EMAIL_VERIFICATION_PURPOSE),
            "Ada@Example.TEST",
        )
        .await
        .expect("issue a bound token");
    let state = store
        .inspect(
            PresentedToken::new(issued.plaintext.expose_secret().to_owned()),
            EMAIL_VERIFICATION_PURPOSE,
        )
        .await
        .expect("inspect")
        .expect("the token");
    assert_eq!(
        state.address_digest.as_deref(),
        Some(address_digest("ada@example.test").as_str())
    );
    assert_eq!(
        address_digest("Ada@Example.TEST"),
        address_digest("ada@example.test")
    );
    assert_ne!(
        address_digest("ada@example.test"),
        address_digest("bob@example.test")
    );

    let plain = store
        .issue(token_input("7", EMAIL_VERIFICATION_PURPOSE))
        .await
        .expect("issue an unbound token");
    let state = store
        .inspect(
            PresentedToken::new(plain.plaintext.expose_secret().to_owned()),
            EMAIL_VERIFICATION_PURPOSE,
        )
        .await
        .expect("inspect")
        .expect("the token");
    assert_eq!(state.address_digest, None);
}

#[tokio::test]
async fn a_binding_without_an_address_column_refuses_to_bind() {
    let db = storage_schema::database().await;
    let store = SeaOrmStorage::<storage_schema::StorageSchema>::new(db.clone());
    let refused = store
        .issue_bound(
            token_input("1", EMAIL_VERIFICATION_PURPOSE),
            "ada@example.test",
        )
        .await;
    assert!(
        matches!(refused, Err(Error::DependencyUnavailable { .. })),
        "{refused:?}"
    );
    let rows: i64 = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) FROM storage_tokens",
        ))
        .await
        .expect("count")
        .expect("a row")
        .try_get_by_index(0)
        .expect("a number");
    assert_eq!(rows, 0, "no token was written");
}

#[tokio::test]
async fn migrate_adds_the_address_column_to_an_older_token_table() {
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    // `auth_tokens` as Magnetar created it before tokens kept an address.
    db.execute_unprepared(
        "CREATE TABLE auth_tokens ( \"id\" integer NOT NULL PRIMARY KEY AUTOINCREMENT, \
         \"user_id\" bigint, \"purpose\" varchar NOT NULL, \"digest\" varchar NOT NULL, \
         \"expires_at\" timestamp_with_timezone_text NOT NULL, \
         \"used_at\" timestamp_with_timezone_text, \
         \"created_at\" timestamp_with_timezone_text, \
         \"updated_at\" timestamp_with_timezone_text )",
    )
    .await
    .expect("the older table");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("upgrade the default tables");
    let store = default_store(&db);
    let issued = store
        .issue_bound(
            token_input("7", EMAIL_VERIFICATION_PURPOSE),
            "ada@example.test",
        )
        .await
        .expect("issue a bound token");
    let state = store
        .inspect(
            PresentedToken::new(issued.plaintext.expose_secret().to_owned()),
            EMAIL_VERIFICATION_PURPOSE,
        )
        .await
        .expect("inspect")
        .expect("the token");
    assert_eq!(
        state.address_digest.as_deref(),
        Some(address_digest("ada@example.test").as_str())
    );
}

#[tokio::test]
async fn mark_email_verified_for_stamps_only_while_the_address_is_the_same() {
    let db = default_database().await;
    let store = default_store(&db);
    let ada = user(&store, "ada@example.test").await;
    assert!(
        !store
            .mark_email_verified_for(&ada.user_id, "bob@example.test", Utc::now())
            .await
            .expect("a refused stamp")
    );
    assert!(!verified(&store, &ada.user_id).await);
    assert!(
        store
            .mark_email_verified_for(&ada.user_id, "ada@example.test", Utc::now())
            .await
            .expect("a stamp")
    );
    assert!(verified(&store, &ada.user_id).await);
}
