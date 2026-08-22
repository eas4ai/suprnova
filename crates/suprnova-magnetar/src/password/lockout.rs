//! Account-level lockout policy over the generic lockout store.
//!
//! Ported from torii's brute-force protection service and the Suprnova
//! facade: threshold-plus-window configuration, atomic failed-attempt
//! post-state, conditional durable lock transitions, and an explicit
//! backend-error policy.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};

use crate::Result;
use crate::storage::{LockoutStore, UserStore};

/// How the lockout check behaves when its storage backend errors.
///
/// The account-lockout check guards the most credential-stuffing-sensitive
/// path in the stack, so the default refuses the request. Deployments that
/// prefer availability during a backend outage opt into `FailOpen`
/// explicitly; the error is traced either way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackendErrorPolicy {
    /// Refuse the request when lockout state cannot be read.
    #[default]
    FailClosed,
    /// Treat unreadable lockout state as unlocked.
    FailOpen,
}

/// Lockout threshold and window configuration.
#[derive(Clone, Copy, Debug)]
pub struct LockoutConfig {
    /// Whether lockout is enforced at all.
    pub enabled: bool,
    /// Failed attempts inside the window that trigger a lock.
    pub max_failed_attempts: u32,
    /// Counting window and lock duration, measured from the latest attempt.
    pub lockout_period: Duration,
    /// Audit retention for attempt rows.
    pub retention_period: Duration,
    /// Behavior when the lockout backend cannot be read.
    pub backend_error_policy: BackendErrorPolicy,
}

impl Default for LockoutConfig {
    /// Torii defaults: enabled, five attempts, fifteen-minute lockout,
    /// seven-day audit retention, fail-closed.
    fn default() -> Self {
        Self {
            enabled: true,
            max_failed_attempts: 5,
            lockout_period: Duration::minutes(15),
            retention_period: Duration::days(7),
            backend_error_policy: BackendErrorPolicy::FailClosed,
        }
    }
}

impl LockoutConfig {
    /// A configuration with lockout disabled entirely.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

/// Computed lockout state for one identity key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockoutStatus {
    /// Attempted identity key (normalized email).
    pub identity: String,
    /// Failed attempts inside the current window.
    pub failed_attempts: u32,
    /// Whether the identity is currently locked.
    pub is_locked: bool,
    /// When the lock lapses, while locked.
    pub locked_until: Option<DateTime<Utc>>,
}

impl LockoutStatus {
    /// Seconds until the lock lapses; `None` when not locked, floor zero.
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<i64> {
        self.locked_until
            .map(|until| (until - Utc::now()).num_seconds().max(0))
    }

    fn unlocked(identity: &str) -> Self {
        Self {
            identity: identity.to_owned(),
            failed_attempts: 0,
            is_locked: false,
            locked_until: None,
        }
    }
}

/// One recorded failure and whether it produced the locked transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedAttempt {
    /// Post-record lockout status.
    pub status: LockoutStatus,
    /// True only for the caller that won the durable unlocked-to-locked
    /// transition. Hosts translate this into their `AccountLocked`
    /// notification.
    pub locked_event: bool,
}

/// Lockout policy service.
pub struct LockoutService {
    store: Arc<dyn LockoutStore>,
    users: Arc<dyn UserStore>,
    config: LockoutConfig,
}

impl LockoutService {
    /// Bind the policy to attempt storage and the user lock column.
    pub fn new(
        store: Arc<dyn LockoutStore>,
        users: Arc<dyn UserStore>,
        config: LockoutConfig,
    ) -> Self {
        Self {
            store,
            users,
            config,
        }
    }

    /// The active configuration.
    #[must_use]
    pub const fn config(&self) -> &LockoutConfig {
        &self.config
    }

    /// Compute the current status for an identity key.
    pub async fn status(&self, identity: &str) -> Result<LockoutStatus> {
        if !self.config.enabled {
            return Ok(LockoutStatus::unlocked(identity));
        }
        let window_start = Utc::now() - self.config.lockout_period;
        let stats = self.store.attempt_stats(identity, window_start).await?;
        Ok(self.compute(identity, stats.count, stats.latest_at))
    }

    /// [`LockoutService::status`] with the configured backend-error policy
    /// applied: `FailOpen` converts a read failure into an unlocked status.
    pub async fn guarded_status(&self, identity: &str) -> Result<LockoutStatus> {
        match self.status(identity).await {
            Ok(status) => Ok(status),
            Err(error) => match self.config.backend_error_policy {
                BackendErrorPolicy::FailClosed => Err(error),
                BackendErrorPolicy::FailOpen => {
                    tracing::error!(
                        error = %error,
                        "lockout backend unreadable; failing open per policy"
                    );
                    Ok(LockoutStatus::unlocked(identity))
                }
            },
        }
    }

    /// Whether the identity is currently locked.
    pub async fn is_locked(&self, identity: &str) -> Result<bool> {
        Ok(self.status(identity).await?.is_locked)
    }

    /// Record one failed attempt and return its exact post-insert status plus
    /// the durable locked-transition signal.
    pub async fn record_failed_attempt(
        &self,
        identity: &str,
        context: Option<&str>,
    ) -> Result<FailedAttempt> {
        if !self.config.enabled {
            return Ok(FailedAttempt {
                status: LockoutStatus::unlocked(identity),
                locked_event: false,
            });
        }
        let at = Utc::now();
        let window_start = at - self.config.lockout_period;
        let stats = self
            .store
            .record_attempt_and_stats(identity, at, context, window_start)
            .await?;
        let status = self.compute(identity, stats.count, stats.latest_at);
        let locked_event = if status.is_locked {
            self.users
                .lock_if_unlocked_by_email(identity, at, window_start)
                .await?
        } else {
            false
        };
        Ok(FailedAttempt {
            status,
            locked_event,
        })
    }

    /// Success-path bookkeeping: clear the counter and the user lock stamp.
    /// Not an admin unlock; no transition signal is produced.
    pub async fn reset_attempts(&self, identity: &str) -> Result<()> {
        self.store.clear_attempts(identity).await?;
        self.users.set_locked_at_by_email(identity, None).await?;
        Ok(())
    }

    /// Admin or reset-path unlock. Returns whether the identity was locked,
    /// so hosts fire their `AccountUnlocked` notification only on a true
    /// transition.
    pub async fn unlock_account(&self, identity: &str) -> Result<bool> {
        let was_locked = self.is_locked(identity).await?;
        self.store.clear_attempts(identity).await?;
        self.users.set_locked_at_by_email(identity, None).await?;
        Ok(was_locked)
    }

    /// Maintenance hook: prune audit rows past the retention period. Hosts
    /// schedule this; the crate spawns no background tasks.
    pub async fn cleanup_expired_attempts(&self) -> Result<u64> {
        self.store
            .cleanup_attempts_before(Utc::now() - self.config.retention_period)
            .await
    }

    fn compute(
        &self,
        identity: &str,
        count: u32,
        latest_at: Option<DateTime<Utc>>,
    ) -> LockoutStatus {
        if count < self.config.max_failed_attempts {
            return LockoutStatus {
                identity: identity.to_owned(),
                failed_attempts: count,
                is_locked: false,
                locked_until: None,
            };
        }
        let locked_until = latest_at.map(|latest| latest + self.config.lockout_period);
        let is_locked = locked_until.is_some_and(|until| until > Utc::now());
        LockoutStatus {
            identity: identity.to_owned(),
            failed_attempts: count,
            is_locked,
            locked_until: if is_locked { locked_until } else { None },
        }
    }
}
