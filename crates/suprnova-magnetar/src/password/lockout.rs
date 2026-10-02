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

/// One atomic verification-attempt admission decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptAdmission {
    /// Whether this caller reserved capacity and may evaluate its proof.
    pub admitted: bool,
    /// Finalized-failure status observed in the same serialized store operation.
    pub status: LockoutStatus,
    /// True when this rejected admission repaired a previously finalized
    /// unlocked-to-locked transition. Hosts publish their lock event from it.
    pub locked_event: bool,
    reservation: Option<AttemptReservationToken>,
}

/// Opaque handle for one pending verification-attempt reservation.
///
/// Hosts must return this handle to [`LockoutService`] to either finalize an
/// invalid proof or cancel an attempt whose verification could not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptReservationToken {
    id: String,
    context: Option<String>,
}

impl AttemptReservationToken {
    /// Construct a token for a store reservation.
    #[must_use]
    pub fn new(id: impl Into<String>, context: Option<String>) -> Self {
        Self {
            id: id.into(),
            context,
        }
    }
}

impl AttemptAdmission {
    /// Reconstruct an admission while adapting it across a host boundary.
    #[must_use]
    pub fn from_parts(
        admitted: bool,
        status: LockoutStatus,
        reservation: Option<AttemptReservationToken>,
    ) -> Self {
        Self {
            admitted,
            status,
            locked_event: false,
            reservation,
        }
    }

    /// Reconstruct an admission including a durable lock-transition signal.
    #[must_use]
    pub fn from_parts_with_event(
        admitted: bool,
        status: LockoutStatus,
        locked_event: bool,
        reservation: Option<AttemptReservationToken>,
    ) -> Self {
        Self {
            admitted,
            status,
            locked_event,
            reservation,
        }
    }

    /// The reservation token when this attempt was admitted.
    #[must_use]
    pub const fn reservation(&self) -> Option<&AttemptReservationToken> {
        self.reservation.as_ref()
    }
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

    /// Atomically reserve capacity before evaluating an authentication proof.
    ///
    /// The admitted reservation is pending until a failed proof finalizes it,
    /// a successful proof removes it with prior finalized failures, or an
    /// aborted verification cancels it. Other in-flight reservations retain
    /// their own lifecycle. Calls beyond the configured threshold insert no row.
    pub async fn admit_attempt(
        &self,
        identity: &str,
        context: Option<&str>,
    ) -> Result<AttemptAdmission> {
        if !self.config.enabled {
            return Ok(AttemptAdmission {
                admitted: true,
                status: LockoutStatus::unlocked(identity),
                locked_event: false,
                reservation: None,
            });
        }
        let at = Utc::now();
        let window_start = at - self.config.lockout_period;
        let reservation = self
            .store
            .admit_attempt_and_stats(
                identity,
                at,
                context,
                window_start,
                self.config.max_failed_attempts,
            )
            .await?;
        let status = self.compute(
            identity,
            reservation.stats.count,
            reservation.stats.latest_at,
        );
        let locked_event = reservation.locked_event;
        let token = reservation
            .reservation_id
            .map(|id| AttemptReservationToken::new(id, context.map(ToOwned::to_owned)));
        Ok(AttemptAdmission {
            admitted: reservation.admitted,
            status,
            locked_event,
            reservation: token,
        })
    }

    /// Cancel exactly one pending reservation after verification aborts.
    ///
    /// Missing or already-finalized reservations are reported as conflicts so
    /// callers fail closed when the attempt state is uncertain.
    pub async fn cancel_attempt(&self, identity: &str, admission: &AttemptAdmission) -> Result<()> {
        let Some(token) = admission.reservation() else {
            return Ok(());
        };
        if self
            .store
            .cancel_attempt_reservation(identity, &token.id)
            .await?
        {
            Ok(())
        } else {
            Err(crate::Error::Conflict {
                resource: "attempt reservation".to_owned(),
                message: "reservation is missing or no longer pending".to_owned(),
            })
        }
    }

    /// Finalize one pending reservation as a failed proof and apply the
    /// durable unlocked-to-locked transition when it reaches the threshold.
    pub async fn finalize_failed_attempt(
        &self,
        identity: &str,
        admission: &AttemptAdmission,
    ) -> Result<FailedAttempt> {
        let Some(token) = admission.reservation() else {
            return Ok(FailedAttempt {
                status: LockoutStatus::unlocked(identity),
                locked_event: false,
            });
        };
        let at = Utc::now();
        let window_start = at - self.config.lockout_period;
        let finalized = self
            .store
            .finalize_attempt_reservation(
                identity,
                &token.id,
                at,
                token.context.as_deref(),
                window_start,
                self.config.max_failed_attempts,
            )
            .await?;
        let status = self.compute(identity, finalized.stats.count, finalized.stats.latest_at);
        Ok(FailedAttempt {
            status,
            locked_event: finalized.locked_event,
        })
    }

    /// Success-path bookkeeping for a proof admitted through
    /// [`Self::admit_attempt`]. The exact pending reservation, prior finalized
    /// failures, and the user lock stamp transition in one storage operation.
    /// Other in-flight pending reservations are preserved. Stores without that
    /// atomic lifecycle fail closed.
    pub async fn reset_admitted_attempts(
        &self,
        identity: &str,
        admission: &AttemptAdmission,
    ) -> Result<()> {
        let Some(token) = admission.reservation() else {
            return Ok(());
        };
        self.store
            .reset_admitted_attempts(identity, &token.id, token.context.as_deref())
            .await?;
        Ok(())
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
