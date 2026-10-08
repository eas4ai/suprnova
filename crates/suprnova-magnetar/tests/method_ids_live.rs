//! Passkey and linked-account rows named by their ids on live PostgreSQL
//! and MySQL/MariaDB.
//!
//! The default schema keys `auth_methods` and `auth_accounts` on BIGINT,
//! while the stores carry those ids as opaque text. PostgreSQL has no
//! implicit text-to-bigint cast, so a store that binds the id as text fails
//! the statement on PostgreSQL only; SQLite and MySQL cast it silently.
//! These tests drive passkey registration, passkey removal and account
//! unlinking through the shipped stores on each live engine.
//!
//! PostgreSQL and MySQL are manual, ignored qualification tests against the
//! live backends named by `MAGNETAR_POSTGRES_TEST_URL` and
//! `MAGNETAR_MYSQL_TEST_URL`. Every test creates users of its own, so
//! repeated runs against one database never collide.

#![cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]

use std::sync::Arc;

use chrono::{Duration, Utc};
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::default_schema::{DefaultAuthSchema, methods};
use magnetar::sessions::{
    OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionMetadata, SessionQueries,
    StoredSession,
};
use magnetar::storage::{
    CredentialActor, LinkedAccountStore, MethodStore, NewLinkedAccount, NewUser, PasskeyStore,
    SeaOrmStorage, UserStore,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, DatabaseConnection};
use sha2::{Digest, Sha256};

struct LiveStore {
    db: DatabaseConnection,
    storage: SeaOrmStorage<DefaultAuthSchema>,
}

async fn live_store(url: &str) -> LiveStore {
    let db = Database::connect(url).await.expect("connect live backend");
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create default auth tables");
    LiveStore {
        storage: SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()),
        db,
    }
}

fn unique(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    format!("{label}-{nanos}-{:016x}", rand::random::<u64>())
}

/// A user with a password, so removing one other method never leaves the
/// account without a way to sign in.
async fn user_with_password(live: &LiveStore) -> String {
    live.storage
        .create_user(NewUser {
            email: format!("{}@example.test", unique("method-ids")),
            password_hash: Some("fixture-hash".to_owned()),
        })
        .await
        .expect("create user")
        .user_id
}

/// A credential actor from a fresh session at the user's current epoch;
/// every method removal moves the epoch on.
async fn actor(live: &LiveStore, user_id: &str) -> CredentialActor {
    let user = live
        .storage
        .find_by_id(user_id)
        .await
        .expect("read user")
        .expect("user exists");
    let token = unique("method-ids-token");
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let store = Arc::new(SqlSessionStore(live.db.clone()));
    store
        .insert_session_if_epoch_current(StoredSession {
            session_id: unique("method-ids-session"),
            user_id: user_id.to_owned(),
            auth_epoch: user.auth_epoch,
            token_hash: digest,
            token_digest: digest,
            expires_at: Utc::now() + Duration::hours(1),
            revoked_at: None,
            metadata: SessionMetadata::default(),
        })
        .await
        .expect("insert a session at the user's current epoch");
    let session = OpaqueSessionProvider::new(store, OpaqueConfig::default())
        .verify_bearer(&token)
        .await
        .expect("the session verifies");
    CredentialActor::from_session(&session)
}

/// Store a passkey row directly, so removal is tested on its own.
async fn seed_passkey(live: &LiveStore, user_id: &str) {
    methods::ActiveModel {
        user_id: Set(user_id.parse().expect("default user ids are integers")),
        credential_id: Set(Some(unique("credential"))),
        public_key: Set(Some("{}".to_owned())),
        created_at: Set(Some(Utc::now())),
        ..Default::default()
    }
    .insert(&live.db)
    .await
    .expect("seed a passkey row");
}

async fn a_registered_passkey_is_read_back_by_its_id(url: &str) {
    let live = live_store(url).await;
    let user_id = user_with_password(&live).await;
    let credential = unique("credential");

    let row = live
        .storage
        .insert_passkey(&actor(&live, &user_id).await, &credential, "{}")
        .await
        .expect("registration reads its row back");

    assert_eq!(row.user_id, user_id);
    assert_eq!(row.credential_id, credential);
    let stored = live.storage.passkeys_for_user(&user_id).await.unwrap();
    assert_eq!(stored, vec![row]);
}

async fn a_passkey_is_removed_by_its_id(url: &str) {
    let live = live_store(url).await;
    let user_id = user_with_password(&live).await;
    seed_passkey(&live, &user_id).await;
    seed_passkey(&live, &user_id).await;
    let before = live.storage.passkeys_for_user(&user_id).await.unwrap();
    assert_eq!(before.len(), 2);

    let removed = live
        .storage
        .remove_passkey_if_not_last(&actor(&live, &user_id).await, &before[0].passkey_id)
        .await
        .expect("removal names the passkey by its id");

    assert!(removed);
    let after = live.storage.passkeys_for_user(&user_id).await.unwrap();
    assert_eq!(after, vec![before[1].clone()]);
}

async fn a_linked_account_is_unlinked_by_its_id(url: &str) {
    let live = live_store(url).await;
    let user_id = user_with_password(&live).await;
    let subject = unique("subject");
    let account = live
        .storage
        .create(
            &actor(&live, &user_id).await,
            NewLinkedAccount {
                user_id: user_id.clone(),
                provider: "github".to_owned(),
                provider_account_id: subject.clone(),
            },
        )
        .await
        .expect("link an account");

    let removed = live
        .storage
        .remove_linked_account_if_not_last(&actor(&live, &user_id).await, &account.account_id)
        .await
        .expect("unlinking names the account by its id");

    assert!(removed);
    assert_eq!(
        live.storage
            .find_by_provider_subject("github", &subject)
            .await
            .unwrap(),
        None
    );
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_registered_passkey_is_read_back_by_its_id() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_registered_passkey_is_read_back_by_its_id(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_passkey_is_removed_by_its_id() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_passkey_is_removed_by_its_id(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn postgres_a_linked_account_is_unlinked_by_its_id() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    a_linked_account_is_unlinked_by_its_id(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_registered_passkey_is_read_back_by_its_id() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_registered_passkey_is_read_back_by_its_id(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_passkey_is_removed_by_its_id() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_passkey_is_removed_by_its_id(&url).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_a_linked_account_is_unlinked_by_its_id() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    a_linked_account_is_unlinked_by_its_id(&url).await;
}
