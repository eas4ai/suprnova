use chrono::{Duration, Utc};
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::sessions::{OpaqueSessionStore, SessionMetadata, StoredSession};
use magnetar::storage::{
    IssueToken, PresentedToken, SeaOrmStorage, TokenStore, UserStore, PASSWORD_RESET_PURPOSE,
};
use secrecy::ExposeSecret;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use sha2::{Digest, Sha256};

mod fixtures;

use fixtures::seaorm_upgrade::{import_fixture, SeaOrm11Fixture};

fn baseline_session_token_digest() -> [u8; 32] {
    Sha256::digest("seaorm11-session-token".as_bytes()).into()
}

async fn verify_baseline_session_and_token(
    database: &DatabaseConnection,
    storage: &SeaOrmStorage<DefaultAuthSchema>,
) {
    let baseline_user = storage
        .find_by_email("seaorm11@example.test")
        .await
        .expect("baseline legacy user should be readable")
        .expect("SeaORM 1.1 fixture must include seaorm11@example.test");

    let baseline_session = SqlSessionStore(database.clone())
        .find_by_token_hash(baseline_session_token_digest())
        .await
        .expect("legacy session row must be queryable")
        .expect("legacy session row must exist");
    assert_eq!(
        baseline_session.user_id, baseline_user.user_id,
        "session must belong to baseline user"
    );

    let token_count: i64 = {
        let row = database
            .query_one_raw(Statement::from_string(
                database.get_database_backend(),
                format!(
                    "SELECT COUNT(*) AS token_count FROM auth_tokens WHERE user_id = {} AND purpose = '{}'",
                    baseline_user.user_id, PASSWORD_RESET_PURPOSE
                ),
            ))
            .await
            .expect("legacy token count query must run")
            .expect("legacy token count row must exist");
        row.try_get_by_index(0)
            .expect("legacy token count must be an integer")
    };
    assert!(token_count >= 1, "fixture must include at least one legacy reset token");

    let token = storage
        .issue(IssueToken {
            user_id: baseline_user.user_id.clone(),
            purpose: PASSWORD_RESET_PURPOSE.to_owned(),
            ttl: std::time::Duration::from_secs(900),
        })
        .await
        .expect("issue a new reset token for compatibility write coverage");
    assert!(storage
        .check(
            PresentedToken::new(token.plaintext.expose_secret()),
            PASSWORD_RESET_PURPOSE,
        )
        .await
        .expect("issued token must be present"));
    storage
        .consume(
            PresentedToken::new(token.plaintext.expose_secret()),
            PASSWORD_RESET_PURPOSE,
        )
        .await
        .expect("issued token must be consumable");

    let session_token_text = format!("seaorm11-upgrade-session-{}", rand::random::<u64>());
    let session_digest: [u8; 32] = Sha256::digest(session_token_text.as_bytes()).into();
    let inserted_session = StoredSession {
        session_id: session_token_text,
        user_id: baseline_user.user_id.clone(),
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
    assert_eq!(stored_session.user_id, baseline_user.user_id);
}

async fn verify_upgrade(fixture: SeaOrm11Fixture) {
    let imported = import_fixture(fixture).await;
    let result = async {
        magnetar::default_schema::migrate(&imported.connection)
            .await
            .expect("run first legacy-schema upgrade migration pass");
        magnetar::default_schema::migrate(&imported.connection)
            .await
            .expect("run second legacy-schema upgrade migration pass");

        let storage = SeaOrmStorage::<DefaultAuthSchema>::new(imported.connection.clone());
        verify_baseline_session_and_token(&imported.connection, &storage).await;

        let _ = storage
            .find_by_email("seaorm11@example.test")
            .await
            .expect("search baseline user again to keep query coverage");

        Ok::<_, magnetar::Error>(())
    }
    .await;
    imported.cleanup().await;
    result.expect("SeaORM 1.1 fixture upgrade flow should be replay-safe and compatible");
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn sqlite_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::Sqlite).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn postgres_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::Postgres).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
async fn mysql_upgrade_from_seaorm_1_1_is_replay_safe() {
    verify_upgrade(SeaOrm11Fixture::MySql).await;
}
