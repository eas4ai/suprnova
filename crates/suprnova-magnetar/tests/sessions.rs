use async_trait::async_trait;
use chrono::{Duration, Utc};
use magnetar::sessions::{
    JwtConfig, JwtEpochStore, JwtSessionProvider, OpaqueSessionProvider, OpaqueSessionStore,
    RememberRow, RememberService, RememberStore, SessionMetadata, SessionQueries, StoredSession,
    WebSessionBinding,
};
use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
#[derive(Default)]
struct MemorySessions(Mutex<Vec<StoredSession>>);

#[async_trait]
impl OpaqueSessionStore for MemorySessions {
    async fn insert_session(&self, session: StoredSession) -> magnetar::Result<()> {
        self.0.lock().push(session);
        Ok(())
    }
    async fn find_by_token_hash(&self, hash: [u8; 32]) -> magnetar::Result<Option<StoredSession>> {
        Ok(self
            .0
            .lock()
            .iter()
            .find(|row| row.token_hash == hash)
            .cloned())
    }
    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<Option<StoredSession>> {
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
        at: chrono::DateTime<Utc>,
    ) -> magnetar::Result<u64> {
        let mut rows = self.0.lock();
        let mut changed = 0;
        for row in rows
            .iter_mut()
            .filter(|row| row.user_id == user_id && row.revoked_at.is_none())
        {
            row.revoked_at = Some(at);
            changed += 1;
        }
        Ok(changed)
    }
    async fn revoke_session(
        &self,
        session_id: &str,
        at: chrono::DateTime<Utc>,
    ) -> magnetar::Result<bool> {
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
        now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<Vec<StoredSession>> {
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
struct Epochs(Mutex<u64>);
#[async_trait]
impl JwtEpochStore for Epochs {
    async fn current_auth_epoch(&self, _user_id: &str) -> magnetar::Result<u64> {
        Ok(*self.0.lock())
    }
    async fn bump_auth_epoch(&self, _user_id: &str) -> magnetar::Result<u64> {
        let mut epoch = self.0.lock();
        *epoch += 1;
        Ok(*epoch)
    }
}

#[derive(Default)]
struct MemoryRemember(Mutex<Vec<RememberRow>>);
#[async_trait]
impl RememberStore for MemoryRemember {
    async fn insert_remember(&self, row: RememberRow) -> magnetar::Result<()> {
        self.0.lock().push(row);
        Ok(())
    }
    async fn find_for_rotation(
        &self,
        selector: &str,
        now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<Option<RememberRow>> {
        Ok(self
            .0
            .lock()
            .iter()
            .find(|row| row.selector == selector && row.expires_at > now)
            .cloned())
    }
    async fn consume_for_rotation(
        &self,
        id: &str,
        selector: &str,
        now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        let mut rows = self.0.lock();
        let index = rows
            .iter()
            .position(|row| row.id == id && row.selector == selector && row.expires_at > now);
        Ok(index
            .map(|index| {
                rows.remove(index);
                true
            })
            .unwrap_or(false))
    }
    async fn revoke_all_remember(&self, user_id: &str) -> magnetar::Result<u64> {
        let mut rows = self.0.lock();
        let before = rows.len();
        rows.retain(|row| row.user_id != user_id);
        Ok((before - rows.len()) as u64)
    }
    async fn prune_expired_remember(&self, now: chrono::DateTime<Utc>) -> magnetar::Result<u64> {
        let mut rows = self.0.lock();
        let before = rows.len();
        rows.retain(|row| row.expires_at > now);
        Ok((before - rows.len()) as u64)
    }
}

#[tokio::test]
async fn opaque_web_binding_uses_digest_and_revocation() {
    let store = Arc::new(MemorySessions::default());
    let expiry = Utc::now() + Duration::hours(1);
    let digest = [7_u8; 32];
    store
        .insert_session(StoredSession {
            session_id: "s1".into(),
            user_id: "u1".into(),
            token_hash: digest,
            token_digest: digest,
            expires_at: expiry,
            revoked_at: None,
            metadata: SessionMetadata::default(),
        })
        .await
        .unwrap();
    let provider =
        OpaqueSessionProvider::new(store.clone(), magnetar::sessions::OpaqueConfig::default());
    let binding = WebSessionBinding {
        session_id: "s1".into(),
        token_digest: digest,
    };
    assert!(provider.verify_bearer(&"07".repeat(32)).await.is_err());
    assert_eq!(provider.revoke_all_for_user("u1").await.unwrap(), 1);
    assert!(
        store
            .find_by_web_binding(&binding)
            .await
            .unwrap()
            .unwrap()
            .revoked_at
            .is_some()
    );
}

#[tokio::test]
async fn jwt_rejects_malformed_tokens() {
    let epochs = Arc::new(Epochs::default());
    let provider = JwtSessionProvider::new(
        JwtConfig::new(
            "magnetar",
            SecretString::from("test-key"),
            Duration::hours(1),
        ),
        epochs,
    )
    .unwrap();
    assert!(provider.verify_bearer("not-a-token").await.is_err());
}

#[tokio::test]
async fn remember_verifier_is_hashed_and_rotates_once() {
    let store = Arc::new(MemoryRemember::default());
    let service = RememberService::new(store.clone(), Duration::days(30)).unwrap();
    let credential = service.issue("u1", Utc::now()).await.unwrap();
    let plaintext = credential.expose_once();
    assert!(
        !store.0.lock()[0]
            .verifier_hash
            .contains(plaintext.expose_secret().split('.').nth(1).unwrap())
    );
    let wrong = plaintext
        .expose_secret()
        .split('.')
        .next()
        .unwrap()
        .to_owned()
        + ".wrong";
    let wrong = magnetar::sessions::RememberCredential::from_host(SecretString::from(wrong));
    assert!(service.rotate(&wrong, Utc::now()).await.is_err());
    assert_eq!(store.0.lock().len(), 1);
    let credential = magnetar::sessions::RememberCredential::from_host(plaintext);
    let (_, replacement) = service.rotate(&credential, Utc::now()).await.unwrap();
    assert!(service.rotate(&credential, Utc::now()).await.is_err());
    assert_eq!(service.revoke_all_for_user("u1").await.unwrap(), 1);
    assert_eq!(
        service
            .prune_expired(Utc::now() + Duration::days(31))
            .await
            .unwrap(),
        0
    );
    assert!(!replacement.expose_once().expose_secret().is_empty());
}
// Compile-time boundary: this generic helper can name only query methods;
// SessionQueries intentionally has no issue/mint operation.
fn _query_only<T: SessionQueries>() {}
