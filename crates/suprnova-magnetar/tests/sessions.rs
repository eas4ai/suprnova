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
    async fn insert_session_if_epoch_current(
        &self,
        session: StoredSession,
    ) -> magnetar::Result<()> {
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

    async fn replace_for_rotation(
        &self,
        id: &str,
        selector: &str,
        now: chrono::DateTime<Utc>,
        replacement: RememberRow,
    ) -> magnetar::Result<bool> {
        let mut rows = self.0.lock();
        let Some(index) = rows
            .iter()
            .position(|row| row.id == id && row.selector == selector && row.expires_at > now)
        else {
            return Ok(false);
        };
        rows[index] = replacement;
        Ok(true)
    }

    async fn revoke_all_remember(&self, user_id: &str) -> magnetar::Result<u64> {
        let mut rows = self.0.lock();
        let before = rows.len();
        rows.retain(|row| row.user_id != user_id);
        Ok((before - rows.len()) as u64)
    }

    async fn revoke_remember_selector(
        &self,
        user_id: &str,
        selector: &str,
    ) -> magnetar::Result<bool> {
        let mut rows = self.0.lock();
        let matching = rows
            .iter()
            .filter(|row| row.user_id == user_id && row.selector == selector)
            .count();
        if matching == 0 {
            return Ok(false);
        }
        if matching > 1 {
            return Err(magnetar::Error::Conflict {
                resource: "remember credential".to_owned(),
                message: "owner and selector matched multiple rows".to_owned(),
            });
        }
        let Some(index) = rows
            .iter()
            .position(|row| row.user_id == user_id && row.selector == selector)
        else {
            return Ok(false);
        };
        rows.remove(index);
        Ok(true)
    }

    async fn prune_expired_remember(&self, now: chrono::DateTime<Utc>) -> magnetar::Result<u64> {
        let mut rows = self.0.lock();
        let before = rows.len();
        rows.retain(|row| row.expires_at > now);
        Ok((before - rows.len()) as u64)
    }
}

#[derive(Default)]
struct LegacyRememberStore(MemoryRemember);

#[async_trait]
impl RememberStore for LegacyRememberStore {
    async fn insert_remember(&self, row: RememberRow) -> magnetar::Result<()> {
        self.0.insert_remember(row).await
    }

    async fn find_for_rotation(
        &self,
        selector: &str,
        now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<Option<RememberRow>> {
        self.0.find_for_rotation(selector, now).await
    }

    async fn consume_for_rotation(
        &self,
        id: &str,
        selector: &str,
        now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        self.0.consume_for_rotation(id, selector, now).await
    }

    async fn revoke_all_remember(&self, user_id: &str) -> magnetar::Result<u64> {
        self.0.revoke_all_remember(user_id).await
    }

    async fn prune_expired_remember(&self, now: chrono::DateTime<Utc>) -> magnetar::Result<u64> {
        self.0.prune_expired_remember(now).await
    }
}

#[tokio::test]
async fn memory_atomic_replace_has_single_winner_and_deterministic_state() {
    let store = Arc::new(MemoryRemember::default());
    let now = Utc::now();
    let original = RememberRow {
        id: "original-id".to_owned(),
        selector: "original-selector".to_owned(),
        user_id: "u1".to_owned(),
        auth_epoch: 7,
        verifier_hash: "sha256:original".to_owned(),
        expires_at: now + Duration::hours(1),
    };
    let first_replacement = RememberRow {
        id: "first-id".to_owned(),
        selector: "first-selector".to_owned(),
        verifier_hash: "sha256:first".to_owned(),
        ..original.clone()
    };
    let second_replacement = RememberRow {
        id: "second-id".to_owned(),
        selector: "second-selector".to_owned(),
        verifier_hash: "sha256:second".to_owned(),
        ..original.clone()
    };
    store.insert_remember(original).await.unwrap();

    let (first_won, second_won) = tokio::join!(
        store.replace_for_rotation(
            "original-id",
            "original-selector",
            now,
            first_replacement.clone(),
        ),
        store.replace_for_rotation(
            "original-id",
            "original-selector",
            now,
            second_replacement.clone(),
        ),
    );
    let first_won = first_won.unwrap();
    let second_won = second_won.unwrap();
    assert_ne!(first_won, second_won, "exactly one rotation must win");

    let rows = store.0.lock().clone();
    let winner = if first_won {
        first_replacement
    } else {
        second_replacement
    };
    assert_eq!(rows, vec![winner]);
}

#[tokio::test]
async fn legacy_remember_store_fails_closed_without_consuming_original() {
    let store = LegacyRememberStore::default();
    let now = Utc::now();
    let original = RememberRow {
        id: "legacy-id".to_owned(),
        selector: "legacy-selector".to_owned(),
        user_id: "legacy-user".to_owned(),
        auth_epoch: 3,
        verifier_hash: "sha256:legacy".to_owned(),
        expires_at: now + Duration::hours(1),
    };
    store.insert_remember(original.clone()).await.unwrap();

    let error = store
        .replace_for_rotation(
            &original.id,
            &original.selector,
            now,
            RememberRow {
                id: "replacement-id".to_owned(),
                selector: "replacement-selector".to_owned(),
                verifier_hash: "sha256:replacement".to_owned(),
                ..original.clone()
            },
        )
        .await
        .expect_err("an old-method-only store must fail closed");
    assert!(matches!(
        error,
        magnetar::Error::DependencyUnavailable {
            dependency,
            message,
        } if dependency == "remember store"
            && message == "atomic remember credential rotation is unavailable"
    ));
    assert_eq!(store.0.0.lock().as_slice(), &[original]);
}

#[tokio::test]
async fn remember_service_revokes_only_the_exact_selector() {
    let store = Arc::new(MemoryRemember::default());
    let service = RememberService::new(store.clone(), Duration::days(30)).unwrap();
    let now = Utc::now();

    let _first = service
        .issue_at_epoch("same-user", 4, now, Duration::days(30))
        .await
        .unwrap();
    let second = service
        .issue_at_epoch("same-user", 4, now, Duration::days(30))
        .await
        .unwrap();
    let selectors = store
        .0
        .lock()
        .iter()
        .map(|row| row.selector.clone())
        .collect::<Vec<_>>();
    assert_eq!(selectors.len(), 2);

    assert!(
        service
            .revoke_selector("same-user", &selectors[0])
            .await
            .unwrap()
    );
    assert!(
        !service
            .revoke_selector("same-user", &selectors[0])
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .0
            .lock()
            .iter()
            .map(|row| row.selector.as_str())
            .collect::<Vec<_>>(),
        vec![selectors[1].as_str()]
    );

    let second = magnetar::sessions::RememberCredential::from_host(second.expose_once());
    let (user_id, _, _) = service
        .rotate_at_epoch(&second, now, Duration::days(30))
        .await
        .expect("the other selector must remain usable");
    assert_eq!(user_id, "same-user");
}

#[tokio::test]
async fn remember_service_does_not_mutate_ambiguous_selector_rows() {
    let store = Arc::new(MemoryRemember::default());
    let service = RememberService::new(store.clone(), Duration::days(30)).unwrap();
    let now = Utc::now();
    let first = RememberRow {
        id: "ambiguous-selector-first".to_owned(),
        selector: "ambiguous-selector".to_owned(),
        user_id: "ambiguous-owner".to_owned(),
        auth_epoch: 2,
        verifier_hash: "sha256:first".to_owned(),
        expires_at: now + Duration::hours(1),
    };
    let second = RememberRow {
        id: "ambiguous-selector-second".to_owned(),
        verifier_hash: "sha256:second".to_owned(),
        ..first.clone()
    };
    store.insert_remember(first.clone()).await.unwrap();
    store.insert_remember(second.clone()).await.unwrap();

    assert!(
        !service
            .revoke_selector("different-owner", &first.selector)
            .await
            .expect("owner mismatch must fail closed")
    );
    assert_eq!(
        store.0.lock().as_slice(),
        &[first.clone(), second.clone()],
        "owner mismatch must not mutate either row"
    );
    let error = service
        .revoke_selector(&first.user_id, &first.selector)
        .await
        .expect_err("ambiguous exact revocation must return an error");
    assert!(matches!(
        error,
        magnetar::Error::Conflict { resource, message }
            if resource == "remember credential"
                && message == "owner and selector matched multiple rows"
    ));
    assert_eq!(
        store.0.lock().as_slice(),
        &[first, second],
        "ambiguous selector revocation must not mutate either row"
    );
}

#[tokio::test]
async fn legacy_remember_store_fails_closed_for_selector_revocation() {
    let store = LegacyRememberStore::default();
    let row = RememberRow {
        id: "legacy-selector-id".to_owned(),
        selector: "legacy-selector-only".to_owned(),
        user_id: "legacy-selector-user".to_owned(),
        auth_epoch: 3,
        verifier_hash: "sha256:legacy-selector".to_owned(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    store.insert_remember(row.clone()).await.unwrap();

    let error = store
        .revoke_remember_selector(&row.user_id, &row.selector)
        .await
        .expect_err("an old-method-only store must fail closed");
    assert!(matches!(
        error,
        magnetar::Error::DependencyUnavailable {
            dependency,
            message,
        } if dependency == "remember store"
            && message == "exact remember credential revocation is unavailable"
    ));
    assert_eq!(store.0.0.lock().as_slice(), &[row]);
}

#[tokio::test]
async fn opaque_web_binding_uses_digest_and_revocation() {
    let store = Arc::new(MemorySessions::default());
    let expiry = Utc::now() + Duration::hours(1);
    let digest = [7_u8; 32];
    store
        .insert_session_if_epoch_current(StoredSession {
            session_id: "s1".into(),
            user_id: "u1".into(),
            auth_epoch: 0,
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
    let credential = service
        .issue_at_epoch("u1", 7, Utc::now(), Duration::days(30))
        .await
        .unwrap();
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
    assert!(
        service
            .rotate_at_epoch(&wrong, Utc::now(), Duration::days(30))
            .await
            .is_err()
    );
    assert_eq!(store.0.lock().len(), 0);
    let credential = service
        .issue_at_epoch("u1", 7, Utc::now(), Duration::days(30))
        .await
        .unwrap()
        .expose_once();
    let credential = magnetar::sessions::RememberCredential::from_host(credential);
    let (_, _, replacement) = service
        .rotate_at_epoch(&credential, Utc::now(), Duration::days(30))
        .await
        .unwrap();
    assert!(
        service
            .rotate_at_epoch(&credential, Utc::now(), Duration::days(30))
            .await
            .is_err()
    );
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
