#![cfg(feature = "seaorm-sqlite")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, DatabaseConnection, EntityTrait};

use super::credential_writes::{CredentialActor, fenced_credential_write};
use super::{SeaOrmStorage, db_error};
use crate::Error;
use crate::default_schema::sql_stores::SqlSessionStore;
use crate::default_schema::{DefaultAuthSchema, users};
use crate::sessions::{
    OpaqueSessionStore, SessionCarrier, SessionMetadata, StoredSession, VerifiedSession,
};

const USER_ID: i64 = 41;
const ISSUANCE_EPOCH: u64 = 7;
const CURRENT_EPOCH: u64 = 8;
const INITIAL_MARKER: &str = "initial";
const COMMITTED_MARKER: &str = "credential-write-committed";
const JWT_COMMITTED_MARKER: &str = "credential-write-jwt-committed";
const ROLLED_BACK_MARKER: &str = "credential-write-rolled-back";
const SESSION_ID: &str = "credential-write-session";

struct Fixture {
    database: DatabaseConnection,
    storage: SeaOrmStorage<DefaultAuthSchema>,
    sessions: SqlSessionStore,
    verified_session: VerifiedSession,
}

async fn fixture() -> Fixture {
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory SQLite");
    crate::default_schema::migrate(&database)
        .await
        .expect("migrate default auth schema");

    users::ActiveModel {
        id: Set(USER_ID),
        email: Set("credential-write@example.test".to_owned()),
        name: Set(Some(INITIAL_MARKER.to_owned())),
        auth_epoch: Set(ISSUANCE_EPOCH as i64),
        ..Default::default()
    }
    .insert(&database)
    .await
    .expect("seed credential owner");

    let expires_at = future_expiry();
    let metadata = SessionMetadata {
        user_agent: Some("credential-write-tests".to_owned()),
        ip_address: Some("192.0.2.41".to_owned()),
    };
    let sessions = SqlSessionStore(database.clone());
    sessions
        .insert_session_if_epoch_current(StoredSession {
            session_id: SESSION_ID.to_owned(),
            user_id: USER_ID.to_string(),
            auth_epoch: ISSUANCE_EPOCH,
            token_hash: [0x41; 32],
            token_digest: [0x42; 32],
            expires_at,
            revoked_at: None,
            metadata: metadata.clone(),
        })
        .await
        .expect("seed opaque session");

    Fixture {
        storage: SeaOrmStorage::new(database.clone()),
        database,
        sessions,
        verified_session: VerifiedSession::new(
            SessionCarrier::Opaque,
            SESSION_ID.to_owned(),
            USER_ID.to_string(),
            ISSUANCE_EPOCH,
            expires_at,
            metadata,
        ),
    }
}

fn future_expiry() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2100-01-01T00:00:00Z")
        .expect("valid fixed expiry")
        .with_timezone(&Utc)
}

fn jwt_verified_session() -> VerifiedSession {
    VerifiedSession::new(
        SessionCarrier::Jwt,
        "credential-write-jwt".to_owned(),
        USER_ID.to_string(),
        ISSUANCE_EPOCH,
        future_expiry(),
        SessionMetadata::default(),
    )
}

async fn set_user_epoch(database: &DatabaseConnection, auth_epoch: u64) {
    users::ActiveModel {
        id: Set(USER_ID),
        auth_epoch: Set(auth_epoch as i64),
        ..Default::default()
    }
    .update(database)
    .await
    .expect("update user authentication epoch");
}

async fn marker(database: &DatabaseConnection) -> Option<String> {
    users::Entity::find_by_id(USER_ID)
        .one(database)
        .await
        .expect("read credential owner")
        .expect("credential owner exists")
        .name
}

fn assert_stale_actor(error: Error) {
    assert_eq!(
        error,
        Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    );
}

#[test]
fn credential_actor_preserves_session_expiry_and_only_opaque_session_ids() {
    let jwt_session = jwt_verified_session();
    let opaque_session = VerifiedSession::new(
        SessionCarrier::Opaque,
        SESSION_ID.to_owned(),
        USER_ID.to_string(),
        ISSUANCE_EPOCH,
        future_expiry(),
        SessionMetadata::default(),
    );

    let jwt_actor = CredentialActor::from_session(&jwt_session);
    let opaque_actor = CredentialActor::from_session(&opaque_session);

    assert_eq!(jwt_actor.opaque_session_id(), None);
    assert_eq!(opaque_actor.opaque_session_id(), Some(SESSION_ID));
    assert_eq!(jwt_actor.expires_at(), Some(future_expiry()));
    assert_eq!(opaque_actor.expires_at(), Some(future_expiry()));
}

#[tokio::test]
async fn credential_fence_rejects_empty_actor_user_id_before_operation() {
    let fixture = fixture().await;
    let malformed_session = VerifiedSession::new(
        SessionCarrier::Jwt,
        "credential-write-empty-user".to_owned(),
        String::new(),
        ISSUANCE_EPOCH,
        future_expiry(),
        SessionMetadata::default(),
    );
    let actor = CredentialActor::from_session(&malformed_session);
    let invocations = Arc::new(AtomicUsize::new(0));

    let error = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |_| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    })
    .await
    .expect_err("an empty actor user id must be rejected before the operation");

    assert_eq!(
        error,
        Error::InvalidInput {
            field: "user_id".to_owned(),
            message: "user identifier must be non-empty".to_owned(),
        }
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(INITIAL_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_accepts_current_opaque_session() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&fixture.verified_session);
    let invocations = Arc::new(AtomicUsize::new(0));

    let committed = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |transaction| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                users::ActiveModel {
                    id: Set(USER_ID),
                    name: Set(Some(COMMITTED_MARKER.to_owned())),
                    ..Default::default()
                }
                .update(transaction.connection())
                .await
                .map_err(db_error)?;
                Ok(41_u64)
            })
        }
    })
    .await
    .expect("current opaque actor may write credentials");

    assert_eq!(committed, 41);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(COMMITTED_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_accepts_current_jwt_epoch() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&jwt_verified_session());
    let invocations = Arc::new(AtomicUsize::new(0));

    fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |transaction| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                users::ActiveModel {
                    id: Set(USER_ID),
                    name: Set(Some(JWT_COMMITTED_MARKER.to_owned())),
                    ..Default::default()
                }
                .update(transaction.connection())
                .await
                .map_err(db_error)?;
                Ok(())
            })
        }
    })
    .await
    .expect("current JWT actor may write credentials");

    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(JWT_COMMITTED_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_rejects_stale_opaque_session_epoch() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&fixture.verified_session);
    set_user_epoch(&fixture.database, CURRENT_EPOCH).await;
    let invocations = Arc::new(AtomicUsize::new(0));

    let error = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |_| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    })
    .await
    .expect_err("stale opaque actor must be rejected");

    assert_stale_actor(error);
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(INITIAL_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_rejects_revoked_opaque_session() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&fixture.verified_session);
    assert!(
        fixture
            .sessions
            .revoke_session(SESSION_ID, Utc::now())
            .await
            .expect("revoke opaque session")
    );
    let invocations = Arc::new(AtomicUsize::new(0));

    let error = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |_| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    })
    .await
    .expect_err("revoked opaque actor must be rejected");

    assert_stale_actor(error);
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(INITIAL_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_rejects_stale_jwt_epoch() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&jwt_verified_session());
    set_user_epoch(&fixture.database, CURRENT_EPOCH).await;
    let invocations = Arc::new(AtomicUsize::new(0));

    let error = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        move |_| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    })
    .await
    .expect_err("stale JWT actor must be rejected");

    assert_stale_actor(error);
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(INITIAL_MARKER)
    );
}

#[tokio::test]
async fn credential_fence_rolls_back_operation_error() {
    let fixture = fixture().await;
    let actor = CredentialActor::from_session(&fixture.verified_session);
    let invocations = Arc::new(AtomicUsize::new(0));
    let deliberate_error = Error::Conflict {
        resource: "credential write test".to_owned(),
        message: "deliberate operation failure".to_owned(),
    };

    let error = fenced_credential_write(&fixture.storage, &actor, {
        let invocations = Arc::clone(&invocations);
        let deliberate_error = deliberate_error.clone();
        move |transaction| {
            Box::pin(async move {
                invocations.fetch_add(1, Ordering::SeqCst);
                users::ActiveModel {
                    id: Set(USER_ID),
                    name: Set(Some(ROLLED_BACK_MARKER.to_owned())),
                    ..Default::default()
                }
                .update(transaction.connection())
                .await
                .map_err(db_error)?;
                Err::<(), Error>(deliberate_error)
            })
        }
    })
    .await
    .expect_err("operation failure must be returned");

    assert_eq!(error, deliberate_error);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
    assert_eq!(
        marker(&fixture.database).await.as_deref(),
        Some(INITIAL_MARKER)
    );
}
