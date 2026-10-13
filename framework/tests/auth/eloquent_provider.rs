//! Integration tests for [`suprnova::EloquentUserProvider`] against a
//! real `#[suprnova::model]` user type in in-memory SQLite.
//!
//! Uses the framework's `TestDatabase` (thread-local connection) + the
//! `#[tokio::test]` current-thread runtime, the established Eloquent
//! integration-test pattern.

use std::any::Any;

use chrono::{DateTime, Utc};
use suprnova::testing::TestDatabase;
use suprnova::{
    Authenticatable, CanResetPassword, Credentials, EloquentUserProvider, MustVerifyEmail,
    UserProvider, model,
};

// The app's `User` shape: a typed model that is also Authenticatable.
// The table carries an extra `is_admin` column the model doesn't map -
// it exists only to prove the credential allowlist never filters on it.
// `email_verified_at` is a nullable datetime - the model macro auto-injects
// `AsOptionalDateTime` on `Option<DateTime<Utc>>` fields, so no explicit cast
// is needed.
#[model(table = "users", fillable = ["email", "password"])]
pub struct TestUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl Authenticatable for TestUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for TestUser {
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

impl CanResetPassword for TestUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

/// Fresh in-memory DB with a `users` table and one seeded user
/// (`a@b.com` / bcrypt(`secret`), not admin). The returned guard must be
/// held for the duration of the test.
async fn setup() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            email_verified_at TEXT, \
            is_admin INTEGER NOT NULL DEFAULT 0\
         )",
    )
    .await
    .unwrap();

    // bcrypt hashes contain `$`, `/`, `.` and alphanumerics - never a
    // single quote - so direct interpolation is safe here.
    let hash = suprnova::hash("secret").unwrap();
    db.execute_unprepared(&format!(
        "INSERT INTO users (email, password, is_admin) VALUES ('a@b.com', '{hash}', 0)"
    ))
    .await
    .unwrap();

    db
}

fn provider() -> EloquentUserProvider<TestUser> {
    EloquentUserProvider::<TestUser>::new()
}

#[tokio::test]
async fn retrieve_by_id_resolves_known_and_unknown() {
    let _db = setup().await;
    let p = provider();

    let user = p.retrieve_by_id("1").await.unwrap().expect("user 1 exists");
    assert_eq!(user.get_auth_identifier(), "1");
    assert!(user.get_auth_password().is_some());

    assert!(p.retrieve_by_id("999").await.unwrap().is_none());
}

#[tokio::test]
async fn retrieve_by_credentials_matches_on_email() {
    let _db = setup().await;
    let p = provider();

    let found = p
        .retrieve_by_credentials(&Credentials::password("a@b.com", "ignored").as_value())
        .await
        .unwrap();
    assert_eq!(
        found.map(|u| u.get_auth_identifier()),
        Some("1".to_string())
    );

    let missing = p
        .retrieve_by_credentials(&Credentials::password("nobody@b.com", "x").as_value())
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn validate_credentials_checks_the_password_hash() {
    let _db = setup().await;
    let p = provider();
    let user = p.retrieve_by_id("1").await.unwrap().unwrap();

    assert!(
        p.validate_credentials(
            &*user,
            &Credentials::password("a@b.com", "secret").as_value()
        )
        .await
        .unwrap()
    );
    assert!(
        !p.validate_credentials(
            &*user,
            &Credentials::password("a@b.com", "wrong").as_value()
        )
        .await
        .unwrap()
    );
}

// A hostile `{email, is_admin: true}` (the seeded user is NOT admin) must
// still resolve by email alone - `is_admin` is not in the allowlist.
#[tokio::test]
async fn credential_allowlist_ignores_non_allowlisted_keys() {
    let _db = setup().await;
    let p = provider();

    let creds = Credentials::new()
        .insert("email", "a@b.com")
        .insert("is_admin", true)
        .as_value();
    let found = p.retrieve_by_credentials(&creds).await.unwrap();
    assert_eq!(
        found.map(|u| u.get_auth_identifier()),
        Some("1".to_string()),
        "is_admin must be ignored; lookup filters on email only"
    );
}

#[tokio::test]
async fn no_allowlisted_credential_returns_none() {
    let _db = setup().await;
    let p = provider();
    let creds = Credentials::new().insert("is_admin", true).as_value();
    assert!(p.retrieve_by_credentials(&creds).await.unwrap().is_none());
}

// The auth-flow surface: lookup-by-email, the AuthFlowUser id round-trip,
// the email-verification toggle, and a password reset - all against the real
// SQLite-backed model, end to end.
#[tokio::test]
async fn eloquent_provider_supports_auth_flow_methods() {
    let _db = setup().await;
    let p = provider();
    let id = "1".to_string();

    // retrieve_by_email returns the AuthFlowUser carrier.
    let found = p
        .retrieve_by_email("a@b.com")
        .await
        .unwrap()
        .expect("user a@b.com exists");
    assert_eq!(found.email, "a@b.com");
    assert_eq!(found.id, id);
    let missing = p.retrieve_by_email("nobody@b.com").await.unwrap();
    assert!(missing.is_none());

    // flow_user_by_id round-trips the email by primary key.
    let by_id = p
        .flow_user_by_id(&id)
        .await
        .unwrap()
        .expect("user 1 exists");
    assert_eq!(by_id.email, "a@b.com");
    assert_eq!(by_id.id, id);

    // Not verified → verified. mark_email_verified persists through save().
    assert!(!p.is_email_verified(&id).await.unwrap());
    p.mark_email_verified(&id).await.unwrap();
    assert!(p.is_email_verified(&id).await.unwrap());

    // set_password stores the pre-hashed value verbatim: the new password
    // verifies, the old one no longer does.
    let new_hash = suprnova::hash("newpass").unwrap();
    p.set_password(&id, &new_hash).await.unwrap();
    let reloaded = p
        .retrieve_by_id(&id)
        .await
        .unwrap()
        .expect("user 1 still exists");
    let stored = reloaded.get_auth_password().expect("password hash present");
    assert!(suprnova::hashing::verify("newpass", stored).unwrap());
    assert!(!suprnova::hashing::verify("secret", stored).unwrap());
}

// The absent-user contract: the mutating flow methods are silent no-ops on a
// non-existent id (load returns None → nothing to mutate, `Ok(())`), and the
// read returns `false`. Locks the behaviour the password-reset / verification
// flows rely on when an id no longer resolves.
#[tokio::test]
async fn auth_flow_methods_are_no_ops_on_absent_user() {
    let _db = setup().await;
    let p = provider();
    let absent = "999";

    // No row → no mutation, no error.
    p.mark_email_verified(absent).await.unwrap();
    p.set_password(absent, &suprnova::hash("whatever").unwrap())
        .await
        .unwrap();

    // Read on a missing id is `false`, not an error.
    assert!(!p.is_email_verified(absent).await.unwrap());

    // The carriers resolve to None for an absent user too.
    assert!(p.flow_user_by_id(absent).await.unwrap().is_none());
}

// ---- Concurrent account flows ----------------------------------------------
//
// A model of its own, so the process-global Updating listener below sees no
// other test's writes.

#[model(table = "race_users", fillable = ["email"])]
pub struct RaceUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl Authenticatable for RaceUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for RaceUser {
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

impl CanResetPassword for RaceUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

/// SQL the next `Updating` of a `RaceUser` runs before its own UPDATE: a
/// second account flow that commits between the provider's read and write.
static CONCURRENT_WRITE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// The two tests share the slot above, so they run one at a time.
static RACE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static RACE_LISTENER: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

struct ConcurrentFlow;

#[async_trait::async_trait]
impl suprnova::eloquent::events::CancellableListener<race_user::events::Updating>
    for ConcurrentFlow
{
    async fn handle(
        &self,
        _event: &race_user::events::Updating,
    ) -> suprnova::eloquent::events::EventResult {
        let sql = CONCURRENT_WRITE.lock().unwrap().take();
        if let Some(sql) = sql {
            use suprnova::sea_orm::ConnectionTrait;
            suprnova::DB::connection()
                .expect("test connection")
                .inner()
                .execute_unprepared(&sql)
                .await
                .expect("concurrent flow write");
        }
        suprnova::eloquent::events::EventResult::Ok
    }
}

async fn race_setup() -> TestDatabase {
    RACE_LISTENER
        .get_or_init(|| async {
            suprnova::eloquent::events::listen_cancellable::<race_user::events::Updating, _>(
                std::sync::Arc::new(ConcurrentFlow),
            )
            .await;
        })
        .await;
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE race_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            email_verified_at TEXT\
         )",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "INSERT INTO race_users (email, password) VALUES ('race@b.com', 'old-hash')",
    )
    .await
    .unwrap();
    db
}

// Email verification loads the user while it still holds the old password
// hash. A password reset stores a new hash before verification writes. The
// verification must write only its own column: writing the whole stale row
// back would make the replaced (perhaps compromised) password valid again.
#[tokio::test]
async fn email_verification_keeps_a_password_changed_while_it_ran() {
    let _serial = RACE_LOCK.lock().await;
    let _db = race_setup().await;
    let p = EloquentUserProvider::<RaceUser>::new();

    *CONCURRENT_WRITE.lock().unwrap() =
        Some("UPDATE race_users SET password = 'new-hash' WHERE id = 1".to_owned());
    p.mark_email_verified("1").await.unwrap();
    assert!(
        CONCURRENT_WRITE.lock().unwrap().is_none(),
        "the concurrent flow ran between the read and the write"
    );

    let stored = <RaceUser as suprnova::Model>::find(1_i64)
        .await
        .unwrap()
        .expect("user 1");
    assert_eq!(stored.password, "new-hash", "the newer password survives");
    assert!(
        stored.email_verified_at.is_some(),
        "the verification landed"
    );
}

// The mirror case: a password reset that loaded the user before a
// verification landed must not write the verification back out.
#[tokio::test]
async fn password_reset_keeps_a_verification_made_while_it_ran() {
    let _serial = RACE_LOCK.lock().await;
    let _db = race_setup().await;
    let p = EloquentUserProvider::<RaceUser>::new();

    *CONCURRENT_WRITE.lock().unwrap() = Some(
        "UPDATE race_users SET email_verified_at = '2026-01-01 00:00:00+00:00' WHERE id = 1"
            .to_owned(),
    );
    p.set_password("1", "reset-hash").await.unwrap();
    assert!(CONCURRENT_WRITE.lock().unwrap().is_none());

    let stored = <RaceUser as suprnova::Model>::find(1_i64)
        .await
        .unwrap()
        .expect("user 1");
    assert_eq!(stored.password, "reset-hash", "the reset landed");
    assert!(
        stored.email_verified_at.is_some(),
        "the newer verification survives"
    );
}

// ---- Password writes independent of mutators and serialization -------------

/// The manual's mutator example: every mass-assigned password is trimmed,
/// checked and hashed.
#[model(
    table = "mutated_users",
    fillable = ["email", "password"],
    mutators = ["password"]
)]
pub struct MutatedUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl MutatedUser {
    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(suprnova::FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}

/// A model that keeps its password hash out of every serialized form.
#[model(table = "quiet_users", fillable = ["email"])]
pub struct QuietUser {
    pub id: i64,
    pub email: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

macro_rules! auth_flow_user {
    ($model:ty) => {
        impl Authenticatable for $model {
            fn get_auth_identifier(&self) -> String {
                self.id.to_string()
            }
            fn get_auth_password(&self) -> Option<&str> {
                Some(&self.password)
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn into_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Any + Send + Sync> {
                self
            }
        }

        impl MustVerifyEmail for $model {
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

        impl CanResetPassword for $model {
            fn email_for_reset(&self) -> &str {
                &self.email
            }
            fn set_password_hash(&mut self, hash: &str) {
                self.password = hash.to_string();
            }
        }
    };
}

auth_flow_user!(MutatedUser);
auth_flow_user!(QuietUser);

async fn user_table(table: &str) -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(&format!(
        "CREATE TABLE {table} (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            email_verified_at TEXT, \
            created_at TEXT, \
            updated_at TEXT\
         )"
    ))
    .await
    .unwrap();
    db.execute_unprepared(&format!(
        "INSERT INTO {table} (email, password) VALUES ('reset@b.com', 'old-hash')"
    ))
    .await
    .unwrap();
    db
}

/// A password reset hands the provider a finished hash. It must be stored
/// as it is: run through the manual's hashing mutator it becomes the hash
/// of a hash, and the new password never works.
#[tokio::test]
async fn a_password_reset_stores_the_hash_verbatim_through_a_mutator() {
    let _db = user_table("mutated_users").await;
    let p = EloquentUserProvider::<MutatedUser>::new();
    let hash = suprnova::hash("brand-new-password").unwrap();

    p.set_password("1", &hash).await.unwrap();

    let stored = <MutatedUser as suprnova::Model>::find(1_i64)
        .await
        .unwrap()
        .expect("user 1");
    assert_eq!(stored.password, hash, "the hash is stored verbatim");
    let user = p.retrieve_by_id("1").await.unwrap().expect("user 1");
    assert!(
        p.validate_credentials(
            &*user,
            &Credentials::password("reset@b.com", "brand-new-password").as_value()
        )
        .await
        .unwrap(),
        "the reset password signs in"
    );
}

/// The write does not depend on how the model serializes: a password the
/// model never serializes is still written.
#[tokio::test]
async fn a_password_reset_writes_a_column_the_model_never_serializes() {
    let _db = user_table("quiet_users").await;
    let p = EloquentUserProvider::<QuietUser>::new();

    p.set_password("1", "new-hash").await.unwrap();
    p.mark_email_verified("1").await.unwrap();

    let stored = <QuietUser as suprnova::Model>::find(1_i64)
        .await
        .unwrap()
        .expect("user 1");
    assert_eq!(stored.password, "new-hash");
    assert!(stored.email_verified_at.is_some());
}

// ---- An encrypted column changed while a password reset ran -----------------

#[model(
    table = "encrypted_users",
    timestamps = false,
    fillable = ["email"],
    casts = { phone = suprnova::AsEncrypted }
)]
pub struct EncryptedUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub phone: String,
    pub email_verified_at: Option<DateTime<Utc>>,
}

impl MustVerifyEmail for EncryptedUser {
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

impl Authenticatable for EncryptedUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn Any + Send + Sync> {
        self
    }
}

impl CanResetPassword for EncryptedUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

/// SQL the next `Updating` of an `EncryptedUser` runs before its UPDATE.
static ENCRYPTED_CONCURRENT_WRITE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static ENCRYPTED_LISTENER: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

struct EncryptedConcurrentFlow;

#[async_trait::async_trait]
impl suprnova::eloquent::events::CancellableListener<encrypted_user::events::Updating>
    for EncryptedConcurrentFlow
{
    async fn handle(
        &self,
        _event: &encrypted_user::events::Updating,
    ) -> suprnova::eloquent::events::EventResult {
        let sql = ENCRYPTED_CONCURRENT_WRITE.lock().unwrap().take();
        if let Some(sql) = sql {
            use suprnova::sea_orm::ConnectionTrait;
            suprnova::DB::connection()
                .expect("test connection")
                .inner()
                .execute_unprepared(&sql)
                .await
                .expect("concurrent flow write");
        }
        suprnova::eloquent::events::EventResult::Ok
    }
}

fn encrypt_cast_value(plain: &str) -> String {
    <suprnova::AsEncrypted as suprnova::Cast>::to_storage(&plain.to_owned()).unwrap()
}

// An encrypted column stores a new ciphertext on every write, so comparing
// stored values counts it as changed even when its value is not. A password
// reset that loaded the user before a concurrent change to that column must
// write only the password, not the stale encrypted value back over it.
#[tokio::test]
async fn a_password_reset_keeps_an_encrypted_column_changed_while_it_ran() {
    if !suprnova::Crypt::is_initialized() {
        suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    }
    ENCRYPTED_LISTENER
        .get_or_init(|| async {
            suprnova::eloquent::events::listen_cancellable::<encrypted_user::events::Updating, _>(
                std::sync::Arc::new(EncryptedConcurrentFlow),
            )
            .await;
        })
        .await;
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE encrypted_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            phone TEXT NOT NULL, \
            email_verified_at TEXT\
         )",
    )
    .await
    .unwrap();
    db.execute_unprepared(&format!(
        "INSERT INTO encrypted_users (email, password, phone) VALUES ('enc@b.com', 'old-hash', '{}')",
        encrypt_cast_value("555-0100")
    ))
    .await
    .unwrap();
    let p = EloquentUserProvider::<EncryptedUser>::new();

    *ENCRYPTED_CONCURRENT_WRITE.lock().unwrap() = Some(format!(
        "UPDATE encrypted_users SET phone = '{}' WHERE id = 1",
        encrypt_cast_value("555-0199")
    ));
    p.set_password("1", "reset-hash").await.unwrap();
    assert!(
        ENCRYPTED_CONCURRENT_WRITE.lock().unwrap().is_none(),
        "the concurrent flow ran between the read and the write"
    );

    let stored = <EncryptedUser as suprnova::Model>::find(1_i64)
        .await
        .unwrap()
        .expect("user 1");
    assert_eq!(stored.password, "reset-hash", "the reset landed");
    assert_eq!(
        stored.phone, "555-0199",
        "the newer encrypted value survives"
    );
}
