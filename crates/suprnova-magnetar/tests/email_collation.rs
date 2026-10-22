//! Address identity must survive MySQL and MariaDB accent-insensitive collations.
//! These engine regressions use an isolated database and the default auth schema.

#![cfg(all(feature = "seaorm-mysql", feature = "magic-link"))]

#[path = "fixtures/live_database.rs"]
mod live_database;

use std::sync::Arc;

use chrono::{Duration, Utc};
use magnetar::auth::{AuthenticationContext, FactorGate, SignInDecision, VerifiedPrincipal};
use magnetar::default_schema::{DefaultAuthSchema, tokens, users};
use magnetar::first_email_proof::{
    FirstEmailProofMutation, FirstEmailProofOutcome, FirstEmailProofStore,
    NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use magnetar::plugins::magic_link::{MagicLinkIssued, MagicLinkService, RegistrationPolicy};
use magnetar::sessions::SessionGrant;
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, Set,
};

use live_database::LiveDatabase;

async fn database(
    label: &str,
) -> (
    LiveDatabase,
    DatabaseConnection,
    Arc<SeaOrmStorage<DefaultAuthSchema>>,
) {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    let live = LiveDatabase::create(&url, label).await;
    let db = live.connect().await;
    magnetar::default_schema::migrate(&db)
        .await
        .expect("create default auth tables");
    // Reproduce the comparison even when the server defaults to an exact collation.
    db.execute_unprepared("ALTER TABLE app_users MODIFY email VARCHAR(254) CHARACTER SET utf8mb4 COLLATE utf8mb4_general_ci NOT NULL")
        .await.expect("use an accent-insensitive email column");
    let store = Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(db.clone()));
    (live, db, store)
}

async fn create_user(store: &impl UserStore, email: &str) -> magnetar::storage::UserRecord {
    store
        .create_user(NewUser {
            email: email.into(),
            password_hash: None,
        })
        .await
        .unwrap()
}

/// Issuance never consumes a proof or signs in, so unexpected calls fail closed.
struct UnusedSignIn;

#[async_trait::async_trait]
impl FirstEmailProofStore for UnusedSignIn {
    async fn apply(&self, _: FirstEmailProofMutation) -> magnetar::Result<FirstEmailProofOutcome> {
        Err(magnetar::Error::Internal {
            message: "issuance must not consume a proof".into(),
        })
    }
    async fn create_verified_provider_account(
        &self,
        _: NewVerifiedProviderAccount,
    ) -> magnetar::Result<VerifiedProviderAccountCommit> {
        Err(magnetar::Error::Internal {
            message: "issuance must not create a provider account".into(),
        })
    }
}

#[async_trait::async_trait]
impl FactorGate for UnusedSignIn {
    async fn complete_sign_in(
        &self,
        _: VerifiedPrincipal,
        _: AuthenticationContext,
    ) -> magnetar::Result<SignInDecision> {
        Err(magnetar::Error::Internal {
            message: "issuance must not sign in".into(),
        })
    }
    async fn complete_challenge(&self, _: &str, _: &str) -> magnetar::Result<SessionGrant> {
        Err(magnetar::Error::Internal {
            message: "issuance must not complete a challenge".into(),
        })
    }
}

#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_confusable_address_finds_no_account() {
    let (live, db, store) = database("email_lookup").await;
    let user = create_user(store.as_ref(), "user@café.example").await;
    assert!(
        store
            .find_by_email("user@cafe.example")
            .await
            .unwrap()
            .is_none()
    );
    for email in ["user@café.example", "USER@café.example"] {
        assert_eq!(
            store.find_by_email(email).await.unwrap().unwrap().user_id,
            user.user_id
        );
    }
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_magic_link_for_a_confusable_address_is_suppressed() {
    let (live, db, store) = database("email_magic").await;
    create_user(store.as_ref(), "user@café.example").await;
    let service = MagicLinkService::new(
        store.clone(),
        store.clone(),
        Arc::new(UnusedSignIn),
        Arc::new(UnusedSignIn),
        RegistrationPolicy::Open,
    );
    assert!(matches!(
        service.issue("user@cafe.example").await.unwrap(),
        MagicLinkIssued::Suppressed
    ));
    assert_eq!(tokens::Entity::find().count(&db).await.unwrap(), 0);
    assert_eq!(users::Entity::find().count(&db).await.unwrap(), 1);
    drop(service);
    drop(store);
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_verification_stamp_refuses_an_accent_variant() {
    let (live, db, store) = database("email_stamp").await;
    let user = create_user(store.as_ref(), "user@cafe.example").await;
    let mut changed: users::ActiveModel =
        users::Entity::find_by_id(user.user_id.parse::<i64>().unwrap())
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
    changed.email = Set("user@café.example".into());
    changed.update(&db).await.unwrap();
    assert!(
        !store
            .mark_email_verified_for(&user.user_id, "user@cafe.example", Utc::now())
            .await
            .unwrap()
    );
    assert!(
        store
            .find_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .email_verified_at
            .is_none()
    );
    assert!(
        store
            .mark_email_verified_for(&user.user_id, "user@café.example", Utc::now())
            .await
            .unwrap()
    );
    assert!(
        store
            .find_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .email_verified_at
            .is_some()
    );
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}

#[tokio::test]
#[ignore = "requires T2 live Postgres/MySQL database"]
async fn mysql_lockout_by_a_confusable_address_locks_nothing() {
    let (live, db, store) = database("email_lockout").await;
    let user = create_user(store.as_ref(), "user@café.example").await;
    let now = Utc::now();
    assert!(
        !store
            .lock_if_unlocked_by_email("user@cafe.example", now, now - Duration::minutes(5))
            .await
            .unwrap()
    );
    store
        .set_locked_at_by_email("user@cafe.example", Some(now))
        .await
        .unwrap();
    assert!(
        store
            .find_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_none()
    );
    // A confusable address must not clear an existing lock either.
    store
        .set_locked_at_by_email("user@café.example", Some(now))
        .await
        .unwrap();
    store
        .set_locked_at_by_email("user@cafe.example", None)
        .await
        .unwrap();
    assert!(
        store
            .find_by_id(&user.user_id)
            .await
            .unwrap()
            .unwrap()
            .locked_at
            .is_some()
    );
    db.close().await.expect("close the isolated database");
    live.drop_database().await;
}
