//! The third-party refresh lease broker and M2M token cache
//! (`docs/specs/suprnova-magnetar/11-token-broker.md`).
//!
//! Composes Task 4's grant executors
//! ([`crate::oauth::grants::refresh`], [`crate::oauth::grants::client_credentials`])
//! and the host transport seam ([`crate::plugin::HttpTransport`]) behind a
//! pre-call lease protocol ([`lease`]) that survives concurrent callers and
//! multiple broker instances sharing one database with no coordination
//! beyond conditional writes. [`singleflight`] is an in-process
//! optimization on top of that protocol, never a correctness requirement.
//!
//! Both a linked account's third-party access/refresh token pair and a
//! cached machine-to-machine (client-credentials) token are "broker
//! records" -- one [`crate::storage::ProviderTokenStore`] row each,
//! addressed by the broker's own opaque `record_id`: a linked-account id
//! for the former, [`M2MCacheKey::record_id`] for the latter. Both go
//! through the identical claim/commit CAS cycle in [`lease`]; only the
//! provider call they make (`refresh_token` grant vs `client_credentials`
//! grant) and their reuse-detection posture differ (M2M records have no
//! "family" to revoke).

pub mod cache;
pub mod lease;
pub mod policy;
pub mod singleflight;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::SecretString;

pub use cache::{M2MCacheConfig, M2MCacheKey};

use crate::crypto::Encryptor;
use crate::oauth::provider::OAuthProviderRegistry;
use crate::plugin::HttpTransport;
use crate::storage::ProviderTokenStore;

/// Configuration for the token broker's lease protocol.
#[derive(Clone, Debug)]
pub struct BrokerConfig {
    /// Whether to coalesce concurrent in-process callers for the same
    /// record onto one lease attempt via [`singleflight::SingleFlight`].
    /// Purely an optimization: the storage CAS protocol in [`lease`] is
    /// unconditionally correct with this `false`.
    pub single_flight: bool,
    /// Upper bound placed on one provider token/client-credentials
    /// exchange. Together with [`Self::lease_grace`] this bounds how long
    /// a claim remains un-reclaimable after a leader starts it.
    pub provider_call_timeout: Duration,
    /// Extra time a lease remains claimed-but-not-yet-reclaimable after
    /// [`Self::provider_call_timeout`] elapses, covering scheduling jitter
    /// and commit latency.
    pub lease_grace: Duration,
    /// How long a follower sleeps between re-reads while waiting on
    /// someone else's live claim or on a stale-generation presenter's
    /// reuse determination.
    pub poll_interval: Duration,
    /// M2M cache freshness policy.
    pub m2m_cache: M2MCacheConfig,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            single_flight: true,
            provider_call_timeout: Duration::from_secs(10),
            lease_grace: Duration::from_secs(5),
            poll_interval: Duration::from_millis(20),
            m2m_cache: M2MCacheConfig::default(),
        }
    }
}

/// A caller-forced refresh request.
///
/// Distinct from [`TokenBroker::access_token`]'s implicit, expiry-driven
/// refresh: a caller here explicitly asserts the refresh-token generation
/// it holds, so a presenter whose generation the store has already moved
/// past is a genuine reuse-detection surface (spec 11's "Refresh under
/// rotation").
#[derive(Clone, Debug)]
pub struct RefreshRequest {
    /// The broker's opaque record identifier.
    pub record_id: String,
    /// The refresh-token generation the caller holds.
    pub presented_generation: i64,
}

/// One access token handed back to a caller, decrypted for immediate use.
pub struct AccessToken {
    /// The bearer access-token value.
    pub value: SecretString,
    /// The token type (ordinarily `Bearer`).
    pub token_type: String,
    /// The token's expiry, when the provider stated one.
    pub expires_at: Option<DateTime<Utc>>,
    /// The granted scopes.
    pub scopes: Vec<String>,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccessToken")
            .field("value", &"[redacted]")
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Fired at most once per detected refresh-token reuse (spec 11's "Refresh
/// under rotation": "the 04 reuse hook fires"). This crate's own
/// lifecycle-hook machinery
/// ([`crate::plugin::LifecycleHook`](crate::plugin::hooks::LifecycleHook))
/// is generic over an [`crate::schema::AuthSchema`] and lives behind the
/// full plugin composition root; [`TokenBroker`] is deliberately
/// non-generic, so this is a narrow, broker-owned hook a host may bridge
/// into its own lifecycle dispatch if it wants one.
#[async_trait]
pub trait ReuseHook: Send + Sync {
    /// Called after the broker has already revoked the record's family.
    async fn on_reuse_detected(&self, record_id: &str, provider: &str);
}

/// A typed terminal-vs-retriable broker failure
/// (`docs/specs/suprnova-magnetar/11-token-broker.md`'s "Upstream error
/// handling": "terminal vs retriable is a typed distinction, not string
/// matching").
#[derive(Debug)]
pub enum BrokerError {
    /// No record exists under this `record_id`.
    NotFound {
        /// The record identifier that was not found.
        record_id: String,
    },
    /// The record's family has been revoked and requires re-authorization.
    Revoked {
        /// The revoked record's identifier.
        record_id: String,
        /// Whether this was a detected-reuse revocation (`true`) or
        /// ordinary dossier-driven revocation (`false`).
        reused: bool,
    },
    /// The record names a provider this broker's registry does not have.
    UnknownProvider {
        /// The unregistered provider name.
        provider: String,
    },
    /// A retriable upstream failure, with the provider's `Retry-After`
    /// when present.
    Retriable {
        /// The provider that failed.
        provider: &'static str,
        /// The failure detail.
        message: String,
        /// The provider's stated retry delay, when present.
        retry_after: Option<Duration>,
    },
    /// A terminal, non-retriable failure.
    Terminal {
        /// The provider that failed, when the failure is provider-
        /// attributable.
        provider: &'static str,
        /// The failure detail.
        message: String,
    },
    /// The lease protocol could not converge within its overall bound
    /// (every observed claim remained live for the whole wait). Not
    /// expected in normal operation; surfaces a stuck/crashed leader whose
    /// claim has not yet reached its deadline.
    LeaseTimeout {
        /// The record whose lease never resolved.
        record_id: String,
    },
    /// An underlying storage failure.
    Storage(crate::Error),
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { record_id } => {
                write!(formatter, "no token broker record '{record_id}'")
            }
            Self::Revoked { record_id, reused } => write!(
                formatter,
                "token broker record '{record_id}' is revoked ({})",
                if *reused {
                    "detected reuse"
                } else {
                    "ordinary revocation"
                }
            ),
            Self::UnknownProvider { provider } => {
                write!(
                    formatter,
                    "token broker provider '{provider}' is not registered"
                )
            }
            Self::Retriable {
                provider,
                message,
                retry_after,
            } => match retry_after {
                Some(delay) => write!(
                    formatter,
                    "token broker provider '{provider}' retriable failure: {message} (retry after {}s)",
                    delay.as_secs()
                ),
                None => write!(
                    formatter,
                    "token broker provider '{provider}' retriable failure: {message}"
                ),
            },
            Self::Terminal { provider, message } => write!(
                formatter,
                "token broker provider '{provider}' terminal failure: {message}"
            ),
            Self::LeaseTimeout { record_id } => write!(
                formatter,
                "token broker record '{record_id}' lease protocol did not converge in time"
            ),
            Self::Storage(error) => write!(formatter, "token broker storage failure: {error}"),
        }
    }
}

impl std::error::Error for BrokerError {}

impl From<crate::Error> for BrokerError {
    fn from(error: crate::Error) -> Self {
        Self::Storage(error)
    }
}

/// The result type used throughout the token broker.
pub type BrokerResult<T> = Result<T, BrokerError>;

/// The token broker's public surface: fresh access tokens for a linked
/// account's third-party grant, a caller-forced refresh, or a cached
/// machine-to-machine token.
#[async_trait]
pub trait TokenBroker: Send + Sync {
    /// Return a fresh access token for `record_id`, refreshing it first
    /// when the stored one is missing or expired. Never requires the
    /// caller to track a generation itself.
    async fn access_token(&self, record_id: &str) -> BrokerResult<AccessToken>;

    /// Force a refresh under an explicitly presented generation. A stale
    /// `presented_generation` is the reuse-detection surface.
    async fn refresh(&self, request: RefreshRequest) -> BrokerResult<AccessToken>;

    /// Return a fresh machine-to-machine access token for `key`,
    /// provisioning and refreshing the cache entry as needed.
    async fn client_credentials(&self, key: M2MCacheKey) -> BrokerResult<AccessToken>;
}

/// The concrete [`TokenBroker`] implementation: Task 4's grant executors
/// plus the host transport seam, behind [`lease`]'s pre-call CAS protocol.
pub struct TokenBrokerService {
    pub(crate) store: Arc<dyn ProviderTokenStore>,
    pub(crate) encryptor: Arc<dyn Encryptor>,
    pub(crate) transport: Arc<dyn HttpTransport>,
    pub(crate) registry: Arc<OAuthProviderRegistry>,
    pub(crate) reuse_hook: Option<Arc<dyn ReuseHook>>,
    pub(crate) config: BrokerConfig,
    pub(crate) coalescing: singleflight::SingleFlight,
}

impl TokenBrokerService {
    /// Compose a broker over application-owned storage, encryption,
    /// transport, and a provider registry.
    pub fn new(
        store: Arc<dyn ProviderTokenStore>,
        encryptor: Arc<dyn Encryptor>,
        transport: Arc<dyn HttpTransport>,
        registry: Arc<OAuthProviderRegistry>,
        config: BrokerConfig,
    ) -> Self {
        Self {
            store,
            encryptor,
            transport,
            registry,
            reuse_hook: None,
            config,
            coalescing: singleflight::SingleFlight::new(),
        }
    }

    /// Attach a reuse hook, fired at most once per detected refresh-token
    /// reuse.
    #[must_use]
    pub fn with_reuse_hook(mut self, hook: Arc<dyn ReuseHook>) -> Self {
        self.reuse_hook = Some(hook);
        self
    }

    /// Idempotently provision a linked-account record at generation zero,
    /// ready for its first [`TokenBroker::access_token`] call to populate.
    /// A host calls this once, right after a linked account's
    /// authorization-code exchange completes, using
    /// [`record_id_for_linked_account`] to derive `record_id`.
    pub async fn provision_linked_account(
        &self,
        record_id: &str,
        provider: &str,
    ) -> BrokerResult<()> {
        self.store
            .create_if_missing(crate::storage::NewProviderToken {
                id: record_id.to_owned(),
                provider: provider.to_owned(),
            })
            .await?;
        Ok(())
    }
}

/// The broker's naming convention for a linked account's provider-token
/// record: the linked-account id itself (each linked account has exactly
/// one third-party token record, per spec 11's "Token records" section).
#[must_use]
pub fn record_id_for_linked_account(linked_account_id: &str) -> String {
    linked_account_id.to_owned()
}

#[async_trait]
impl TokenBroker for TokenBrokerService {
    async fn access_token(&self, record_id: &str) -> BrokerResult<AccessToken> {
        lease::access_token(self, record_id).await
    }

    async fn refresh(&self, request: RefreshRequest) -> BrokerResult<AccessToken> {
        lease::refresh(self, request).await
    }

    async fn client_credentials(&self, key: M2MCacheKey) -> BrokerResult<AccessToken> {
        lease::client_credentials(self, key).await
    }
}
