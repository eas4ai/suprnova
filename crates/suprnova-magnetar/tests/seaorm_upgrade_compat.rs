use chrono::{Duration, Utc};
use futures_util::future::FutureExt;
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::sessions::{OpaqueSessionStore, SessionMetadata, StoredSession};
use magnetar::storage::{
    IssueToken, NewUser, PresentedToken, SeaOrmStorage, TokenStore, UserStore, PASSWORD_RESET_PURPOSE,
};
use sea_orm::DatabaseConnection;
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};
use std::panic::AssertUnwindSafe;

mod fixtures {
    pub mod seaorm_upgrade;
}

use fixtures::seaorm_upgrade::{import_fixture, SeaOrm11Fixture};

const BASELINE_EMAIL: &str = "seaorm11@example.test";
const BASELINE_USER_ID: &str = "6100";
const BASELINE_SESSION_ID: &str = "seaorm11-session";
const BASELINE_SESSION_TOKEN: &str = "seaorm11-session-token";
const BASELINE_LEGACY_TOKEN: &str = "seaorm11-token";
const BASELINE_AUTH_EPOCH: u64 = 0;
const BASELINE_WRITE_EMAIL: &str = "seaorm11-upgrade-write@example.test";

fn baseline_session_token_digest() -> [u8; 32] {
    Sha256::digest(BASELINE_SESSION_TOKEN.as_bytes()).into()
}

async fn verify_baseline_session_and_token(
    database: &DatabaseConnection,
    storage: &SeaOrmStorage<DefaultAuthSchema>,
) {
    let baseline_user = storage
        .find_by_email(BASELINE_EMAIL)
        .await
        .expect("baseline legacy user should be readable")
        .expect("SeaORM 1.1 fixture must include seaorm11@example.test");

    assert_eq!(
        baseline_user.user_id, BASELINE_USER_ID,
        "seaorm 1.1 fixture should preserve user id"
    );
    assert_eq!(
        baseline_user.auth_epoch, BASELINE_AUTH_EPOCH,
        "baseline user should preserve auth_epoch"
    );

    let baseline_session = SqlSessionStore(database.clone())
        .find_by_token_hash(baseline_session_token_digest())
        .await
        .expect("legacy session row must be queryable")
        .expect("baseline session fixture row should exist");

    assert_eq!(
        baseline_session.auth_epoch, BASELINE_AUTH_EPOCH,
        "baseline session should preserve auth_epoch"
    );
    assert_eq!(
        baseline_session.user_id, baseline_user.user_id,
        "session must belong to baseline user"
    );
    assert_eq!(
        baseline_session.session_id, BASELINE_SESSION_ID,
        "session id should be preserved"
    );

    assert!(storage
        .check(PresentedToken::new(BASELINE_LEGACY_TOKEN), PASSWORD_RESET_PURPOSE)
        .await
        .expect("legacy reset token should be present"));

    let consumed = storage
        .consume(PresentedToken::new(BASELINE_LEGACY_TOKEN), PASSWORD_RESET_PURPOSE)
        .await
        .expect("legacy reset token must be consumable");
    assert_eq!(
        consumed.user_id.as_deref(),
        Some(BASELINE_USER_ID),
        "consumed legacy token should map to baseline user"
    );

    let issued = storage
        .issue(IssueToken {
            user_id: baseline_user.user_id.clone(),
            purpose: PASSWORD_RESET_PURPOSE.to_owned(),
            ttl: std::time::Duration::from_secs(900),
        })
        .await
        .expect("issue a new reset token for compatibility write coverage");
    assert!(storage
        .check(
            PresentedToken::new(issued.plaintext.expose_secret()),
            PASSWORD_RESET_PURPOSE,
        )
        .await
        .expect("issued token must remain present"));
    let consumed_issue = storage
        .consume(
            PresentedToken::new(issued.plaintext.expose_secret()),
            PASSWORD_RESET_PURPOSE,
        )
        .await
        .expect("issued token should be consumable");
    assert_eq!(
        consumed_issue.user_id.as_deref(),
        Some(BASELINE_USER_ID),
        "issued token must consume to baseline user"
    );

    let create_user = storage
        .create_user(NewUser {
            email: BASELINE_WRITE_EMAIL.to_owned(),
            password_hash: None,
        })
        .await
        .expect("token and session flow should still allow fresh user writes");
    assert_ne!(
        create_user.user_id, BASELINE_USER_ID,
        "newly created users must not reuse frozen legacy identifier"
    );

    let session_token_text = format!("seaorm11-upgrade-session-{}", rand::random::<u64>());
    let session_digest: [u8; 32] = Sha256::digest(session_token_text.as_bytes()).into();
    let inserted_session = StoredSession {
        session_id: session_token_text,
        user_id: baseline_user.user_id,
        auth_epoch: baseline_user.auth_epoch,
        token_digest: session_digest,
        token_hash: session_digest,
        expires_at: Utc::now() + Duration::hours(1),
        revoked_at: None,
        metadata: SessionMetadata::default(),
    };
    SqlSessionStore(database.clone())
        .insert_session_if_epoch_current(inserted_session)
        .await
        .expect("test session insertion should succeed");
    let stored_session = SqlSessionStore(database.clone())
        .find_by_token_hash(session_digest)
        .await
        .expect("stored session should be queryable")
        .expect("stored session should be present");
    assert_eq!(stored_session.user_id, BASELINE_USER_ID);
}

async fn verify_upgrade(fixture: SeaOrm11Fixture) {
    let imported = import_fixture(fixture).await.expect("fixture import should succeed");

    let run = AssertUnwindSafe(async {
        magnetar::default_schema::migrate(&imported.connection)
            .await
            .expect("run first legacy-schema upgrade migration pass");
        magnetar::default_schema::migrate(&imported.connection)
            .await
            .expect("run second legacy-schema upgrade migration pass");

        let storage = SeaOrmStorage::<DefaultAuthSchema>::new(imported.connection.clone());
        verify_baseline_session_and_token(&imported.connection, &storage).await;

        let _ = storage
            .find_by_email(BASELINE_EMAIL)
            .await
            .expect("search baseline user again to keep query coverage");

        Ok::<_, magnetar::Error>(())
    })
    .catch_unwind()
    .await;

    imported
        .cleanup()
        .await
        .expect("fixture cleanup must run after migration compatibility verification");

    match run {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("SeaORM 1.1 fixture upgrade flow failed: {error}"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::Sqlite).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
#[ignore = "manual live PostgreSQL qualification"]
async fn postgres_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::Postgres).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
#[ignore = "manual live MySQL qualification"]
async fn mysql_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::MySql).await;
}
