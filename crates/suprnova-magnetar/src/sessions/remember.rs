//! Selector-plus-verifier remember-me credentials.
//!
//! The returned composite is a host carrier input; cookie encryption remains
//! outside Magnetar. Only the selector and a one-way verifier hash are stored.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use bcrypt::verify;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use crate::storage::UserStore;

use super::{Result, expired, invalid};

/// A row persisted by a remember-me store.
#[derive(Clone, PartialEq, Eq)]
pub struct RememberRow {
    /// Generated row identifier used for conditional rotation deletes.
    pub id: String,
    /// O(1)-lookup selector, normally protected by a unique index.
    pub selector: String,
    /// Owning user identifier.
    pub user_id: String,
    /// Authentication epoch observed when this credential was issued.
    pub auth_epoch: u64,
    /// One-way verifier hash. New rows use `sha256:<hex>`; legacy bcrypt rows remain readable.
    pub verifier_hash: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for RememberRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RememberRow")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("auth_epoch", &self.auth_epoch)
            .field("verifier_scheme", &verifier_scheme(&self.verifier_hash))
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
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
pub struct RememberCredential(SecretString);

impl fmt::Debug for RememberCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RememberCredential([REDACTED])")
    }
}

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

/// Classification of a security-relevant remember credential anomaly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RememberAnomalyKind {
    /// No active row resolved for the presented selector, including replay.
    UnknownOrReusedSelector,
    /// A valid selector resolved but its verifier did not match.
    VerifierMismatch,
}

/// Redacted remember credential anomaly delivered to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RememberAnomaly {
    /// Stable anomaly classification. No selector or verifier is retained.
    pub kind: RememberAnomalyKind,
    /// Resolved owner only when a valid selector exposed a verifier mismatch.
    pub user_id: Option<String>,
}

/// Host hook for security-relevant remember credential anomalies.
#[async_trait]
pub trait RememberAnomalyHook: Send + Sync {
    /// Observe one already-redacted anomaly.
    async fn on_anomaly(&self, anomaly: RememberAnomaly);
}

struct IgnoreRememberAnomalies;

#[async_trait]
impl RememberAnomalyHook for IgnoreRememberAnomalies {
    async fn on_anomaly(&self, _anomaly: RememberAnomaly) {}
}

/// Erased low-level selector-token service consumed only by the policy-aware service.
#[async_trait]
pub trait RememberTokenService: Send + Sync {
    /// Explicit default lifetime used by core/plugin callers that do not override it.
    fn default_lifetime(&self) -> Duration;
    /// Issue for an already-loaded user epoch and explicit lifetime.
    async fn issue_at_epoch(
        &self,
        user_id: &str,
        auth_epoch: u64,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential>;
    /// Rotate and preserve the issuance epoch with an explicit replacement lifetime.
    async fn rotate_at_epoch(
        &self,
        credential: &RememberCredential,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<(String, u64, RememberCredential)>;
    /// Revoke all remember rows for one user.
    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64>;
}

/// Low-level selector+verifier token service.
///
/// This type never loads users and therefore cannot issue without an explicit
/// authentication epoch. Route and plugin callers must use [`RememberSignInService`].
pub struct RememberService<S: ?Sized> {
    store: Arc<S>,
    default_lifetime: Duration,
    anomaly_hook: Arc<dyn RememberAnomalyHook>,
}

impl<S: ?Sized> Clone for RememberService<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            default_lifetime: self.default_lifetime,
            anomaly_hook: Arc::clone(&self.anomaly_hook),
        }
    }
}

impl<S: RememberStore + ?Sized> RememberService<S> {
    /// Bind the service to storage and an explicit core/plugin default lifetime.
    pub fn new(store: Arc<S>, default_lifetime: Duration) -> Result<Self> {
        validate_lifetime(default_lifetime)?;
        Ok(Self {
            store,
            default_lifetime,
            anomaly_hook: Arc::new(IgnoreRememberAnomalies),
        })
    }

    /// Install a redacted anomaly hook.
    #[must_use]
    pub fn with_anomaly_hook(mut self, hook: Arc<dyn RememberAnomalyHook>) -> Self {
        self.anomaly_hook = hook;
        self
    }

    /// Issue with an already-loaded user epoch and explicit lifetime.
    pub async fn issue_at_epoch(
        &self,
        user_id: &str,
        auth_epoch: u64,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential> {
        validate_lifetime(lifetime)?;
        if user_id.is_empty() {
            return Err(invalid("user_id", "must not be empty"));
        }
        let selector = random_hex::<16>();
        let verifier = random_hex::<32>();
        let verifier_hash = sha256_verifier(&verifier);
        self.store
            .insert_remember(RememberRow {
                id: random_hex::<16>(),
                selector: selector.clone(),
                user_id: user_id.to_owned(),
                auth_epoch,
                verifier_hash,
                expires_at: now + lifetime,
            })
            .await?;
        Ok(RememberCredential::from_host(SecretString::from(format!(
            "{selector}.{verifier}"
        ))))
    }

    /// Atomically rotate one presented credential and issue its replacement
    /// with an explicit lifetime while preserving its issuance epoch.
    pub async fn rotate_at_epoch(
        &self,
        credential: &RememberCredential,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<(String, u64, RememberCredential)> {
        validate_lifetime(replacement_lifetime)?;
        let (selector, verifier) = parse(credential.0.expose_secret())?;
        let Some(row) = self.store.find_for_rotation(&selector, now).await? else {
            self.anomaly_hook
                .on_anomaly(RememberAnomaly {
                    kind: RememberAnomalyKind::UnknownOrReusedSelector,
                    user_id: None,
                })
                .await;
            return Err(expired("remember token"));
        };
        if !verifier_matches(&verifier, &row.verifier_hash)? {
            self.store.revoke_all_remember(&row.user_id).await?;
            self.anomaly_hook
                .on_anomaly(RememberAnomaly {
                    kind: RememberAnomalyKind::VerifierMismatch,
                    user_id: Some(row.user_id.clone()),
                })
                .await;
            return Err(invalid("credential", "invalid verifier"));
        }
        if !self
            .store
            .consume_for_rotation(&row.id, &row.selector, now)
            .await?
        {
            self.anomaly_hook
                .on_anomaly(RememberAnomaly {
                    kind: RememberAnomalyKind::UnknownOrReusedSelector,
                    user_id: None,
                })
                .await;
            return Err(expired("remember token"));
        }
        let replacement = self
            .issue_at_epoch(&row.user_id, row.auth_epoch, now, replacement_lifetime)
            .await?;
        Ok((row.user_id, row.auth_epoch, replacement))
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

#[async_trait]
impl<S: RememberStore + ?Sized> RememberTokenService for RememberService<S> {
    fn default_lifetime(&self) -> Duration {
        self.default_lifetime
    }

    async fn issue_at_epoch(
        &self,
        user_id: &str,
        auth_epoch: u64,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential> {
        RememberService::issue_at_epoch(self, user_id, auth_epoch, now, lifetime).await
    }

    async fn rotate_at_epoch(
        &self,
        credential: &RememberCredential,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<(String, u64, RememberCredential)> {
        RememberService::rotate_at_epoch(self, credential, now, replacement_lifetime).await
    }

    async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        RememberService::revoke_all_for_user(self, user_id).await
    }
}

/// Successful remembered primary authentication.
#[derive(Debug)]
pub struct RememberSignInOutcome {
    /// Fresh opaque session issued through the factor gate.
    pub session: super::SessionGrant,
    /// Sole live replacement for the consumed remember credential.
    pub replacement: RememberCredential,
}

/// Policy-aware remembered primary-authentication service.
pub struct RememberSignInService<U> {
    remember: Arc<dyn RememberTokenService>,
    users: Arc<U>,
    factor_gate: Arc<dyn FactorGate>,
}

impl<U> Clone for RememberSignInService<U> {
    fn clone(&self) -> Self {
        Self {
            remember: Arc::clone(&self.remember),
            users: Arc::clone(&self.users),
            factor_gate: Arc::clone(&self.factor_gate),
        }
    }
}

impl<U: UserStore> RememberSignInService<U> {
    /// Compose policy-aware remembered authentication around an erased token service.
    pub fn new(
        remember: Arc<dyn RememberTokenService>,
        users: Arc<U>,
        factor_gate: Arc<dyn FactorGate>,
    ) -> Self {
        Self {
            remember,
            users,
            factor_gate,
        }
    }

    /// Issue at the current user epoch using the explicit core default lifetime.
    pub async fn issue(&self, user_id: &str, now: DateTime<Utc>) -> Result<RememberCredential> {
        self.issue_with_lifetime(user_id, now, self.remember.default_lifetime())
            .await
    }

    /// Issue at the current user epoch with a route-supplied lifetime.
    pub async fn issue_with_lifetime(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential> {
        let user = self.current_user(user_id).await?;
        self.remember
            .issue_at_epoch(&user.user_id, user.auth_epoch, now, lifetime)
            .await
    }

    /// Sign in using the explicit core default replacement lifetime.
    pub async fn sign_in(
        &self,
        credential: RememberCredential,
        metadata: super::SessionMetadata,
        now: DateTime<Utc>,
    ) -> Result<RememberSignInOutcome> {
        self.sign_in_with_lifetime(credential, metadata, now, self.remember.default_lifetime())
            .await
    }

    /// Sign in and rotate with a route-supplied replacement lifetime.
    pub async fn sign_in_with_lifetime(
        &self,
        credential: RememberCredential,
        metadata: super::SessionMetadata,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<RememberSignInOutcome> {
        let (user_id, issued_epoch, replacement) = self
            .remember
            .rotate_at_epoch(&credential, now, replacement_lifetime)
            .await?;
        let user = self.current_user(&user_id).await?;
        if user.auth_epoch != issued_epoch {
            return Err(invalid(
                "credential",
                "invalid or stale remember credential",
            ));
        }
        let context = AuthenticationContext::new(metadata, user.auth_epoch, now);
        let principal =
            VerifiedPrincipal::new(user.user_id, SignInMethod::Remembered, context.clone())?;
        match self
            .factor_gate
            .complete_sign_in(principal, context)
            .await?
        {
            SignInDecision::SessionAllowed(session) => Ok(RememberSignInOutcome {
                session,
                replacement,
            }),
            SignInDecision::FactorRequired { .. } => Err(crate::Error::Internal {
                message: "remembered proof unexpectedly required a second factor".to_owned(),
            }),
        }
    }

    /// Revoke every remember credential for a user.
    pub async fn revoke_all_for_user(&self, user_id: &str) -> Result<u64> {
        self.remember.revoke_all_for_user(user_id).await
    }

    async fn current_user(&self, user_id: &str) -> Result<crate::storage::UserRecord> {
        self.users
            .find_by_id(user_id)
            .await?
            .ok_or_else(|| crate::Error::NotFound {
                resource: "user".to_owned(),
                identifier: user_id.to_owned(),
            })
    }
}

/// Object-safe policy-aware remember boundary consumed by plugins and hosts.
#[async_trait]
pub trait RememberFacade: Send + Sync {
    /// Issue at the current user epoch using the configured core default lifetime.
    async fn issue_now(&self, user_id: &str) -> Result<RememberCredential>;
    /// Issue at the current user epoch with an explicit lifetime.
    async fn issue_with_lifetime(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential>;
    /// Consume, rotate, and sign in with an explicit replacement lifetime.
    async fn sign_in_with_lifetime(
        &self,
        credential: RememberCredential,
        metadata: super::SessionMetadata,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<RememberSignInOutcome>;
    /// Revoke all remember-me rows for a user.
    async fn revoke_all(&self, user_id: &str) -> Result<u64>;
}

#[async_trait]
impl<U: UserStore> RememberFacade for RememberSignInService<U> {
    async fn issue_now(&self, user_id: &str) -> Result<RememberCredential> {
        self.issue(user_id, Utc::now()).await
    }

    async fn issue_with_lifetime(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
        lifetime: Duration,
    ) -> Result<RememberCredential> {
        RememberSignInService::issue_with_lifetime(self, user_id, now, lifetime).await
    }

    async fn sign_in_with_lifetime(
        &self,
        credential: RememberCredential,
        metadata: super::SessionMetadata,
        now: DateTime<Utc>,
        replacement_lifetime: Duration,
    ) -> Result<RememberSignInOutcome> {
        RememberSignInService::sign_in_with_lifetime(
            self,
            credential,
            metadata,
            now,
            replacement_lifetime,
        )
        .await
    }

    async fn revoke_all(&self, user_id: &str) -> Result<u64> {
        self.revoke_all_for_user(user_id).await
    }
}

fn validate_lifetime(lifetime: Duration) -> Result<()> {
    if lifetime <= Duration::zero() {
        return Err(invalid("lifetime", "must be positive"));
    }
    Ok(())
}

fn verifier_scheme(hash: &str) -> &'static str {
    if hash.starts_with("sha256:") {
        "sha256"
    } else if hash.starts_with("$2") {
        "bcrypt-legacy"
    } else {
        "unknown"
    }
}

fn sha256_verifier(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    format!("sha256:{}", encode_hex(&digest))
}

fn verifier_matches(verifier: &str, stored: &str) -> Result<bool> {
    if let Some(expected) = stored.strip_prefix("sha256:") {
        let candidate = Sha256::digest(verifier.as_bytes());
        let candidate = encode_hex(&candidate);
        return Ok(expected.as_bytes().ct_eq(candidate.as_bytes()).into());
    }
    if stored.starts_with("$2") {
        return verify(verifier, stored).map_err(|error| crate::Error::Internal {
            message: format!("verify legacy remember credential: {error}"),
        });
    }
    Ok(false)
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
    encode_hex(&bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
