#![cfg(feature = "seaorm-sqlite")]

#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use magnetar::storage::{IssueToken, PresentedToken, SeaOrmStorage, TokenStore};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::time::Duration;
use storage_schema::{StorageSchema, database};
#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn wrong_purpose_rolls_back_and_sibling_invalidation_is_atomic() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    let first = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "email".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    let sibling = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "email".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    assert!(
        store
            .consume(
                PresentedToken::new(first.plaintext.expose_secret().to_owned()),
                "wrong"
            )
            .await
            .is_err()
    );
    let trigger = format!(
        "CREATE TRIGGER fail_sibling BEFORE UPDATE OF used_at ON storage_tokens \
         WHEN OLD.id = {} BEGIN SELECT RAISE(ABORT, 'blocked sibling'); END",
        sibling.token_id
    );
    db.execute(Statement::from_string(DbBackend::Sqlite, trigger))
        .await
        .unwrap();
    assert!(
        store
            .consume(
                PresentedToken::new(first.plaintext.expose_secret().to_owned()),
                "email"
            )
            .await
            .is_err()
    );
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER fail_sibling",
    ))
    .await
    .unwrap();
    let consumed = store
        .consume(
            PresentedToken::new(first.plaintext.expose_secret().to_owned()),
            "email",
        )
        .await
        .unwrap();
    assert_eq!(consumed.purpose, "email");
    assert!(
        store
            .consume(
                PresentedToken::new(sibling.plaintext.expose_secret().to_owned()),
                "email"
            )
            .await
            .is_err()
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn selector_digest_is_single_use() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let issued = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "reset".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    let token = issued.plaintext.expose_secret().to_owned();
    assert!(
        store
            .consume(PresentedToken::new(token.clone()), "reset")
            .await
            .is_ok()
    );
    assert!(
        store
            .consume(PresentedToken::new(token), "reset")
            .await
            .is_err()
    );
}

use secrecy::ExposeSecret;

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn password_reset_advances_epoch_and_revokes_sessions() {
    use magnetar::storage::{PasswordResetInput, PasswordResetStore};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
    use storage_schema::sessions;
    let db = database().await;
    sessions::ActiveModel {
        id: Set("1".into()),
        user_id: Set(1),
        token_digest: Set("session".into()),
        token_hash: Set(None),
        user_agent: Set(None),
        ip_address: Set(None),
        expires_at: Set(chrono::Utc::now() + chrono::Duration::minutes(5)),
        revoked_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    let issued = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "password-reset".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    let result = store
        .apply_password_reset(
            PasswordResetInput::new(
                PresentedToken::new(issued.plaintext.expose_secret().to_owned()),
                "new-hash",
            )
            .expecting_user("1"),
        )
        .await
        .unwrap();
    assert_eq!(result.auth_epoch, 1);
    assert_eq!(result.revoked_sessions, 1);
    assert!(
        sessions::Entity::find_by_id("1")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .revoked_at
            .is_some()
    );
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn password_reset_wrong_user_rolls_back_token_and_epoch() {
    use magnetar::storage::{PasswordResetInput, PasswordResetStore};
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let issued = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "password-reset".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    let token = issued.plaintext.expose_secret().to_owned();
    assert!(
        store
            .apply_password_reset(
                PasswordResetInput::new(PresentedToken::new(token.clone()), "new-hash")
                    .expecting_user("2"),
            )
            .await
            .is_err()
    );
    assert!(
        store
            .consume(PresentedToken::new(token), "password-reset")
            .await
            .is_ok()
    );
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn configured_postgres_target_is_required() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL must be configured");
    sea_orm::Database::connect(url)
        .await
        .expect("configured PostgreSQL target must connect");
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
async fn configured_mysql_target_is_required() {
    let url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
        .expect("MAGNETAR_MYSQL_TEST_URL must be configured");
    sea_orm::Database::connect(url)
        .await
        .expect("configured MySQL target must connect");
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn racing_consumers_have_one_token_winner() {
    let db = database().await;
    let store = SeaOrmStorage::<StorageSchema>::new(db);
    let issued = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "race".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    let token = issued.plaintext.expose_secret().to_owned();
    let (left, right) = tokio::join!(
        store.consume(PresentedToken::new(token.clone()), "race"),
        store.consume(PresentedToken::new(token), "race")
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn password_reset_session_failure_rolls_back_epoch_credential_and_token() {
    use magnetar::storage::{PasswordResetInput, PasswordResetStore};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
    use storage_schema::{sessions, users};
    let db = database().await;
    sessions::ActiveModel {
        id: Set("1".into()),
        user_id: Set(1),
        token_digest: Set("session".into()),
        token_hash: Set(None),
        user_agent: Set(None),
        ip_address: Set(None),
        expires_at: Set(chrono::Utc::now() + chrono::Duration::minutes(5)),
        revoked_at: Set(None),
    }
    .insert(&db)
    .await
    .unwrap();
    let store = SeaOrmStorage::<StorageSchema>::new(db.clone());
    let issued = store
        .issue(IssueToken {
            user_id: "1".into(),
            purpose: "password-reset".into(),
            ttl: Duration::from_secs(60),
        })
        .await
        .unwrap();
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TRIGGER fail_session BEFORE UPDATE OF revoked_at ON storage_sessions BEGIN SELECT RAISE(ABORT, 'blocked session'); END",
    ))
    .await
    .unwrap();
    let token = issued.plaintext.expose_secret().to_owned();
    assert!(
        store
            .apply_password_reset(
                PasswordResetInput::new(PresentedToken::new(token.clone()), "new-hash")
                    .expecting_user("1"),
            )
            .await
            .is_err()
    );
    db.execute(Statement::from_string(
        DbBackend::Sqlite,
        "DROP TRIGGER fail_session",
    ))
    .await
    .unwrap();
    let user = users::Entity::find_by_id(1)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.auth_epoch, 0);
    assert_eq!(user.password_hash.as_deref(), Some("old"));
    assert!(
        sessions::Entity::find_by_id("1")
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .revoked_at
            .is_none()
    );
    assert!(
        store
            .consume(PresentedToken::new(token), "password-reset")
            .await
            .is_ok()
    );
}
