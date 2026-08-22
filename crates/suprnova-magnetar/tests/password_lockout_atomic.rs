//! Deterministic lockout concurrency regressions.

#![cfg(feature = "password")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use magnetar::password::{LockoutConfig, LockoutService};
use magnetar::storage::{
    AttemptStats, CredentialActor, LockoutStore, NewUser, UserRecord, UserStore,
};
use parking_lot::Mutex;
use tokio::sync::Barrier;

/// An attempt store that releases both threshold contenders only after both
/// inserts are visible. This exposes split record-then-read implementations
/// without relying on scheduler timing.
struct BarrierAttemptStore {
    recorded: Mutex<Vec<DateTime<Utc>>>,
    both_recorded: Barrier,
}

impl BarrierAttemptStore {
    fn one_below_threshold() -> Self {
        Self {
            recorded: Mutex::new(vec![Utc::now()]),
            both_recorded: Barrier::new(2),
        }
    }

    fn empty() -> Self {
        Self {
            recorded: Mutex::new(Vec::new()),
            both_recorded: Barrier::new(2),
        }
    }

    fn count(&self) -> usize {
        self.recorded.lock().len()
    }

    fn age_all_by(&self, duration: Duration) {
        for attempted_at in self.recorded.lock().iter_mut() {
            *attempted_at -= duration;
        }
    }
}

#[async_trait]
impl LockoutStore for BarrierAttemptStore {
    async fn record_attempt_and_stats(
        &self,
        _identity: &str,
        at: DateTime<Utc>,
        _context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        let stats = {
            let mut recorded = self.recorded.lock();
            recorded.push(at);
            let mut in_window = recorded
                .iter()
                .filter(|attempted_at| **attempted_at >= window_start)
                .copied()
                .collect::<Vec<_>>();
            in_window.sort_unstable();
            AttemptStats {
                count: u32::try_from(in_window.len()).unwrap(),
                latest_at: in_window.last().copied(),
            }
        };
        self.both_recorded.wait().await;
        Ok(stats)
    }
    async fn attempt_stats(
        &self,
        _identity: &str,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<AttemptStats> {
        let mut in_window = self
            .recorded
            .lock()
            .iter()
            .filter(|attempted_at| **attempted_at >= window_start)
            .copied()
            .collect::<Vec<_>>();
        in_window.sort_unstable();
        Ok(AttemptStats {
            count: u32::try_from(in_window.len()).unwrap(),
            latest_at: in_window.last().copied(),
        })
    }

    async fn clear_attempts(&self, _identity: &str) -> magnetar::Result<u64> {
        let removed = self.recorded.lock().drain(..).count();
        Ok(u64::try_from(removed).unwrap())
    }

    async fn cleanup_attempts_before(&self, before: DateTime<Utc>) -> magnetar::Result<u64> {
        let mut recorded = self.recorded.lock();
        let before_count = recorded.len();
        recorded.retain(|attempted_at| *attempted_at >= before);
        Ok(u64::try_from(before_count - recorded.len()).unwrap())
    }
}

/// A transition sink that counts durable lock stamps. An atomic record +
/// post-state result must cause exactly one unlocked-to-locked write even when
/// multiple callers cross the threshold together.
#[derive(Default)]
struct TransitionUserStore {
    lock_transitions: AtomicUsize,
    lock_timestamps: Mutex<Vec<DateTime<Utc>>>,
}

impl TransitionUserStore {
    fn age_locks_by(&self, duration: Duration) {
        for locked_at in self.lock_timestamps.lock().iter_mut() {
            *locked_at -= duration;
        }
    }
}

#[async_trait]
impl UserStore for TransitionUserStore {
    async fn find_by_email(&self, _email: &str) -> magnetar::Result<Option<UserRecord>> {
        Ok(None)
    }

    async fn find_by_id(&self, _user_id: &str) -> magnetar::Result<Option<UserRecord>> {
        Ok(None)
    }

    async fn create_user(&self, input: NewUser) -> magnetar::Result<UserRecord> {
        Ok(UserRecord {
            user_id: "unused-user".to_owned(),
            email: input.email,
            password_hash: input.password_hash,
            email_verified_at: None,
            locked_at: None,
            auth_epoch: 0,
        })
    }

    async fn set_password_hash(
        &self,
        _actor: &CredentialActor,
        _password_hash: &str,
    ) -> magnetar::Result<()> {
        Ok(())
    }

    async fn mark_email_verified(
        &self,
        _user_id: &str,
        _at: DateTime<Utc>,
    ) -> magnetar::Result<()> {
        Ok(())
    }

    async fn lock_if_unlocked_by_email(
        &self,
        _email: &str,
        locked_at: DateTime<Utc>,
        window_start: DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        let mut lock_timestamps = self.lock_timestamps.lock();
        let eligible = match lock_timestamps.last() {
            Some(previous) => *previous < window_start,
            None => true,
        };
        if eligible {
            lock_timestamps.push(locked_at);
            self.lock_transitions.fetch_add(1, Ordering::SeqCst);
        }
        Ok(eligible)
    }
    async fn set_locked_at_by_email(
        &self,
        _email: &str,
        locked_at: Option<DateTime<Utc>>,
    ) -> magnetar::Result<()> {
        let mut lock_timestamps = self.lock_timestamps.lock();
        if let Some(locked_at) = locked_at {
            lock_timestamps.push(locked_at);
            self.lock_transitions.fetch_add(1, Ordering::SeqCst);
        } else {
            lock_timestamps.clear();
        }
        Ok(())
    }
}

#[tokio::test]
async fn concurrent_threshold_attempts_use_atomic_post_state_and_emit_one_lock_transition() {
    let attempts = Arc::new(BarrierAttemptStore::one_below_threshold());
    let users = Arc::new(TransitionUserStore::default());
    let service = LockoutService::new(
        attempts.clone(),
        users.clone(),
        LockoutConfig {
            max_failed_attempts: 2,
            ..LockoutConfig::default()
        },
    );

    let (first, second) = tokio::join!(
        service.record_failed_attempt("contended@example.test", Some("198.51.100.1")),
        service.record_failed_attempt("contended@example.test", Some("198.51.100.2")),
    );
    let outcomes = [first.unwrap(), second.unwrap()];
    let mut post_counts = outcomes
        .iter()
        .map(|outcome| outcome.status.failed_attempts)
        .collect::<Vec<_>>();
    post_counts.sort_unstable();

    assert_eq!(
        attempts.count(),
        3,
        "the seeded failure and both concurrent failures must each be recorded exactly once"
    );
    assert_eq!(
        post_counts,
        [2, 3],
        "each atomic insert must return its own exact post-insert count"
    );
    assert!(
        outcomes.iter().all(|outcome| outcome.status.is_locked),
        "both post-insert states are at or above the threshold"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.locked_event)
            .count(),
        1,
        "the threshold crossing must emit one locked event"
    );
    assert_eq!(
        users.lock_transitions.load(Ordering::SeqCst),
        1,
        "the post-state transition bit must produce one durable lock transition"
    );
}

#[tokio::test]
async fn an_expired_window_can_emit_exactly_one_new_lock_transition() {
    let attempts = Arc::new(BarrierAttemptStore::empty());
    let users = Arc::new(TransitionUserStore::default());
    let service = LockoutService::new(
        attempts.clone(),
        users.clone(),
        LockoutConfig {
            max_failed_attempts: 2,
            lockout_period: Duration::hours(1),
            ..LockoutConfig::default()
        },
    );
    let identity = "later-cycle@example.test";

    let (first, second) = tokio::join!(
        service.record_failed_attempt(identity, Some("198.51.100.10")),
        service.record_failed_attempt(identity, Some("198.51.100.11")),
    );
    let first_cycle = [first.unwrap(), second.unwrap()];
    assert_eq!(
        first_cycle
            .iter()
            .filter(|outcome| outcome.locked_event)
            .count(),
        1,
        "the first threshold crossing emits one transition"
    );
    let first_locked_at = users.lock_timestamps.lock()[0];

    // Expire the entire first counting window without reset/unlock. The
    // durable user lock timestamp remains present but belongs to the old
    // cycle.
    attempts.age_all_by(Duration::hours(2));
    users.age_locks_by(Duration::hours(2));
    let stale_locked_at = users.lock_timestamps.lock()[0];
    assert!(stale_locked_at < first_locked_at);
    assert!(!service.status(identity).await.unwrap().is_locked);
    assert_eq!(users.lock_timestamps.lock().as_slice(), [stale_locked_at]);

    let (third, fourth) = tokio::join!(
        service.record_failed_attempt(identity, Some("198.51.100.12")),
        service.record_failed_attempt(identity, Some("198.51.100.13")),
    );
    let second_cycle = [third.unwrap(), fourth.unwrap()];
    assert_eq!(
        second_cycle
            .iter()
            .filter(|outcome| outcome.locked_event)
            .count(),
        1,
        "a later window must emit exactly one fresh transition"
    );
    let lock_timestamps = users.lock_timestamps.lock();
    assert_eq!(
        lock_timestamps.len(),
        2,
        "the durable lock timestamp must be stamped once per lock cycle"
    );
    assert!(
        lock_timestamps[1] > stale_locked_at,
        "the later cycle must advance the durable lock timestamp"
    );
}
