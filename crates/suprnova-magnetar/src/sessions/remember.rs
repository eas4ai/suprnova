//! Selector-plus-verifier remember-me credentials.
//!
//! The returned composite is a host carrier input; cookie encryption remains
//! outside Magnetar. Only the selector and a one-way verifier hash are stored.

use async_trait::async_trait;
use bcrypt::{DEFAULT_COST, hash, verify};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

use super::{Result, expired, invalid};

/// A row persisted by a remember-me store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RememberRow {
    /// Generated row identifier used for conditional rotation deletes.
    pub id: String,
    /// O(1)-lookup selector, normally protected by a unique index.
    pub selector: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Bcrypt hash of the verifier; plaintext never reaches storage.
    pub verifier_hash: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Persistence boundary for selector+verifier rows.
#[async_trait]
pub trait RememberStore: Send + Sync {
    /// Insert one selector row.
    async fn insert_remember(&self, row: RememberRow) -> Result<()>;
    /// Look up one active selector row without consuming it.
    async fn find_for_rotation(
        &self,
        selector: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<RememberRow>>;
    /// Conditionally consume exactly one still-valid row by id and selector.
    async fn consume_for_rotation(
        &self,
        id: &str,
        selector: &str,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    /// Revoke all rows for a user.
    async fn revoke_all_remember(&self, user_id: &str) -> Result<u64>;
    /// Prune expired rows and return the number removed.
    async fn prune_expired_remember(&self, now: DateTime<Utc>) -> Result<u64>;
}
/// Host-side remember-me carrier. It should be encrypted before cookie storage.
#[derive(Debug)]
pub struct RememberCredential(SecretString);

impl RememberCredential {
    /// Wrap a host-decrypted composite before verification.
    ///
    /// Cookie encryption/decryption belongs to the host; Magnetar only
    /// receives the resulting secret value and never persists it.
    pub fn from_host(value: SecretString) -> Self {
        Self(value)
    }
    /// Consume and expose the composite to the host encryption adapter.
    pub fn expose_once(self) -> SecretString {
        self.0
    }
}
/// Selector+verifier remember-me service.
pub struct RememberService<S> {
    store: Arc<S>,
    lifetime: Duration,
}

impl<S> Clone for RememberService<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            lifetime: self.lifetime,
        }
    }
}

impl<S: RememberStore> RememberService<S> {
    /// Bind the service to application-owned remember-me storage.
    pub fn new(store: Arc<S>, lifetime: Duration) -> Result<Self> {
        if lifetime <= Duration::zero() {
            return Err(invalid("lifetime", "must be positive"));
        }
        Ok(Self { store, lifetime })
    }

    /// Issue a selector+verifier credential for a user.
    pub async fn issue(&self, user_id: &str, now: DateTime<Utc>) -> Result<RememberCredential> {
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        let selector = random_hex::<16>();
        let verifier = random_hex::<32>();
        let verifier_hash =
            hash(&verifier, DEFAULT_COST).map_err(|error| crate::Error::Internal {
                message: error.to_string(),
            })?;
        self.store
            .insert_remember(RememberRow {
                id: random_hex::<16>(),
                selector: selector.clone(),
                user_id: user_id.to_owned(),
                verifier_hash,
                expires_at: now + self.lifetime,
            })
            .await?;
        Ok(RememberCredential::from_host(SecretString::from(format!(
            "{selector}.{verifier}"
        ))))
    }

    /// Atomically rotate a presented credential. A concurrent caller can win
    /// only once because the conditional consume operation races on id+selector.
    pub async fn rotate(
        &self,
        credential: &RememberCredential,
        now: DateTime<Utc>,
    ) -> Result<(String, RememberCredential)> {
        let (selector, verifier) = parse(credential.0.expose_secret())?;
        let row = self
            .store
            .find_for_rotation(&selector, now)
            .await?
            .ok_or_else(|| expired("remember token"))?;
        let valid = verify(&verifier, &row.verifier_hash)
            .map_err(|_| invalid("credential", "invalid verifier"))?;
        if !valid {
            return Err(invalid("credential", "invalid verifier"));
        }
        if !self
            .store
            .consume_for_rotation(&row.id, &row.selector, now)
            .await?
        {
            return Err(expired("remember token"));
        }
        let replacement = self.issue(&row.user_id, now).await?;
        Ok((row.user_id, replacement))
    }

    /// Revoke all remember-me rows for a user.
    pub async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        self.store.revoke_all_remember(user_id).await
    }

    /// Remove expired rows as a maintenance operation.
    pub async fn prune_expired(&self, now: DateTime<Utc>) -> Result<u64> {
        self.store.prune_expired_remember(now).await
    }
}

/// Object-safe remember-me boundary consumed by plugins.
///
/// [`RememberService`] is generic over its store; route plugins hold this
/// erased facade instead so hosts can compose any [`RememberStore`].
#[async_trait]
pub trait RememberFacade: Send + Sync {
    /// Issue a selector+verifier credential for a user at the current time.
    async fn issue_now(&self, user_id: &str) -> Result<RememberCredential>;
    /// Revoke all remember-me rows for a user.
    async fn revoke_all(&self, user_id: &str) -> Result<u64>;
}

#[async_trait]
impl<S: RememberStore> RememberFacade for RememberService<S> {
    async fn issue_now(&self, user_id: &str) -> Result<RememberCredential> {
        self.issue(user_id, Utc::now()).await
    }

    async fn revoke_all(&self, user_id: &str) -> Result<u64> {
        self.revoke_all_for_user(user_id).await
    }
}

fn parse(value: &str) -> Result<(String, String)> {
    let mut parts = value.split('.');
    let selector = parts.next().unwrap_or_default();
    let verifier = parts.next().unwrap_or_default();
    if parts.next().is_some() || selector.is_empty() || verifier.is_empty() {
        return Err(invalid("credential", "expected selector.verifier"));
    }
    Ok((selector.to_owned(), verifier.to_owned()))
}

fn random_hex<const N: usize>() -> String {
    let mut bytes = [0_u8; N];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
