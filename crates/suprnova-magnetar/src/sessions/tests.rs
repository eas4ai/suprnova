use super::*;
use chrono::Duration;
use parking_lot::Mutex;
use secrecy::ExposeSecret;
use std::sync::Arc;

#[derive(Default)]
struct Store(Mutex<Vec<StoredSession>>);

#[async_trait::async_trait]
impl OpaqueSessionStore for Store {
    async fn insert_session_if_epoch_current(&self, row: StoredSession) -> Result<()> {
        self.0.lock().push(row);
        Ok(())
    }
    async fn find_by_token_hash(&self, digest: [u8; 32]) -> Result<Option<StoredSession>> {
        Ok(self
            .0
            .lock()
            .iter()
            .find(|row| row.token_hash == digest)
            .cloned())
    }
    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> Result<Option<StoredSession>> {
        Ok(self
            .0
            .lock()
            .iter()
            .find(|row| {
                row.session_id == binding.session_id && row.token_digest == binding.token_digest
            })
            .cloned())
    }
    async fn revoke_all_sessions(
        &self,
        user_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64> {
        let mut rows = self.0.lock();
        let mut n = 0;
        for row in rows
            .iter_mut()
            .filter(|row| row.user_id == user_id && row.revoked_at.is_none())
        {
            row.revoked_at = Some(at);
            n += 1;
        }
        Ok(n)
    }
    async fn revoke_session(
        &self,
        session_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        let mut rows = self.0.lock();
        if let Some(row) = rows
            .iter_mut()
            .find(|row| row.session_id == session_id && row.revoked_at.is_none())
        {
            row.revoked_at = Some(at);
            return Ok(true);
        }
        Ok(false)
    }
    async fn list_active_sessions(
        &self,
        user_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<StoredSession>> {
        Ok(self
            .0
            .lock()
            .iter()
            .filter(|row| {
                row.user_id == user_id && row.expires_at > now && row.revoked_at.is_none()
            })
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct Epoch(Mutex<u64>);
#[async_trait::async_trait]
impl JwtEpochStore for Epoch {
    async fn current_auth_epoch(&self, _user_id: &str) -> Result<u64> {
        Ok(*self.0.lock())
    }
    async fn bump_auth_epoch(&self, _user_id: &str) -> Result<u64> {
        let mut epoch = self.0.lock();
        *epoch += 1;
        Ok(*epoch)
    }
}

#[tokio::test]
async fn issuer_grants_are_one_time_carriers_and_web_bindings_are_host_gated() {
    let store = Arc::new(Store::default());
    let provider = OpaqueSessionProvider::new(
        store.clone(),
        OpaqueConfig {
            lifetime: Duration::hours(1),
        },
    );
    let issuer = SessionIssuer;
    let grant = issuer
        .issue_opaque(
            &provider,
            SessionIssuer::approval(0),
            "u1".into(),
            SessionMetadata::default(),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let binding = grant.web_binding();
    assert_eq!(binding.token_digest, store.0.lock()[0].token_digest);
    let bearer = grant.into_bearer();
    let token = bearer.expose_token_once();
    assert!(!token.expose_secret().is_empty());
    let host = HostSessionApproval::authenticated();
    assert_eq!(
        provider
            .resolve_web_binding(&binding, &host)
            .await
            .unwrap()
            .user_id(),
        "u1"
    );
    assert!(provider.verify_bearer(&"00".repeat(32)).await.is_err());
    assert_eq!(provider.revoke_all_for_user("u1").await.unwrap(), 1);
    assert!(provider.resolve_web_binding(&binding, &host).await.is_err());
}

#[tokio::test]
async fn opaque_session_round_trip_preserves_issuance_epoch() {
    let store = Arc::new(Store::default());
    let token_digest = [0x5a; 32];
    let expires_at = chrono::DateTime::<chrono::Utc>::MAX_UTC;
    store
        .insert_session_if_epoch_current(StoredSession {
            session_id: "session-with-issuance-epoch".to_owned(),
            user_id: "user-with-issuance-epoch".to_owned(),
            auth_epoch: 41,
            token_hash: token_digest,
            token_digest,
            expires_at,
            revoked_at: None,
            metadata: SessionMetadata::default(),
        })
        .await
        .unwrap();
    let provider = OpaqueSessionProvider::new(
        store,
        OpaqueConfig {
            lifetime: Duration::hours(1),
        },
    );
    let binding = WebSessionBinding {
        session_id: "session-with-issuance-epoch".to_owned(),
        token_digest,
    };
    let verified = provider
        .resolve_web_binding(&binding, &HostSessionApproval::authenticated())
        .await
        .unwrap();

    assert_eq!(verified.auth_epoch(), 41);
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn opaque_session_issuance_rejects_stale_gate_approval() {
    use crate::default_schema::{migrate, sessions, sql_stores::SqlSessionStore, users};
    use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};

    let database = Database::connect("sqlite::memory:").await.unwrap();
    migrate(&database).await.unwrap();
    users::ActiveModel {
        id: Set(1),
        email: Set("stale-session@example.test".to_owned()),
        auth_epoch: Set(1),
        ..Default::default()
    }
    .insert(&database)
    .await
    .unwrap();
    let provider = OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(database.clone())),
        OpaqueConfig {
            lifetime: Duration::hours(1),
        },
    );

    let result = SessionIssuer
        .issue_opaque(
            &provider,
            SessionIssuer::approval(0),
            "1".to_owned(),
            SessionMetadata::default(),
            chrono::Utc::now(),
        )
        .await;
    let error = match result {
        Ok(_) => panic!("stale gate approval must not mint an opaque session"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        crate::Error::NotFound {
            resource: "credential actor".to_owned(),
            identifier: "expired or revoked".to_owned(),
        }
    );
    assert!(
        sessions::Entity::find()
            .all(&database)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn jwt_issuance_rejects_stale_gate_approval() {
    let epochs = Arc::new(Epoch::default());
    let provider = JwtSessionProvider::new(
        JwtConfig::new(
            "issuer-a",
            secrecy::SecretString::from("key-a"),
            Duration::hours(1),
        ),
        epochs.clone(),
    )
    .unwrap();
    let approval = SessionIssuer::approval(0);
    epochs.bump_auth_epoch("u1").await.unwrap();

    let result = SessionIssuer
        .issue_jwt(
            &provider,
            approval,
            "u1".into(),
            SessionMetadata::default(),
            chrono::Utc::now(),
        )
        .await;
    let error = match result {
        Ok(_) => panic!("stale gate approval must not mint a JWT"),
        Err(error) => error,
    };

    assert!(matches!(error, crate::Error::InvalidInput { field, .. } if field == "auth_epoch"));
}

#[tokio::test]
async fn jwt_issuance_checks_epoch_and_issuer() {
    let epochs = Arc::new(Epoch::default());
    let provider = JwtSessionProvider::new(
        JwtConfig::new(
            "issuer-a",
            secrecy::SecretString::from("key-a"),
            Duration::hours(1),
        ),
        epochs.clone(),
    )
    .unwrap();
    let grant = SessionIssuer
        .issue_jwt(
            &provider,
            SessionIssuer::approval(0),
            "u1".into(),
            SessionMetadata::default(),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let token = grant.into_bearer().expose_token_once();
    let verified = provider.verify_bearer(token.expose_secret()).await.unwrap();
    assert_eq!(verified.user_id(), "u1");
    epochs.bump_auth_epoch("u1").await.unwrap();
    assert!(provider.verify_bearer(token.expose_secret()).await.is_err());
    let fresh_epochs = Arc::new(Epoch::default());
    let other = JwtSessionProvider::new(
        JwtConfig::new(
            "issuer-b",
            secrecy::SecretString::from("key-a"),
            Duration::hours(1),
        ),
        fresh_epochs,
    )
    .unwrap();
    let error = other
        .verify_bearer(token.expose_secret())
        .await
        .unwrap_err();
    assert!(matches!(error, crate::Error::InvalidInput { field, .. } if field == "issuer"));
}
