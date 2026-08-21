use std::{collections::HashMap, sync::Mutex, time::Duration};

use async_trait::async_trait;
use magnetar::{
    Error, Result,
    abuse::{AbuseLimiter, AbusePolicy, Permit},
};

#[derive(Default)]
struct DeterministicLimiter {
    counts: Mutex<HashMap<String, u32>>,
    backend_error: bool,
}

impl DeterministicLimiter {
    fn failing() -> Self {
        Self {
            counts: Mutex::new(HashMap::new()),
            backend_error: true,
        }
    }
}

#[async_trait]
impl AbuseLimiter for DeterministicLimiter {
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit> {
        policy.validate()?;
        if self.backend_error {
            return Err(Error::DependencyUnavailable {
                dependency: "test-backend".to_owned(),
                message: "deterministic backend failure".to_owned(),
            });
        }
        let mut counts = self.counts.lock().expect("test lock must not be poisoned");
        let count = counts.entry(key.to_owned()).or_default();
        *count += 1;
        if *count > policy.max_requests {
            Ok(Permit::Rejected {
                retry_after: policy.window,
            })
        } else {
            Ok(Permit::Allowed {
                retry_after: Some(policy.window),
            })
        }
    }
}

fn policy() -> AbusePolicy {
    AbusePolicy {
        max_requests: 1,
        window: Duration::from_secs(60),
    }
}

#[test]
fn policy_rejects_sub_millisecond_windows_for_redis_precision() {
    let invalid = AbusePolicy {
        max_requests: 1,
        window: Duration::from_nanos(1),
    };
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn present_and_absent_identities_have_identical_outcomes() {
    let limiter = DeterministicLimiter::default();
    let present = limiter
        .acquire("password-reset\0known@example.test", policy())
        .await
        .expect("present identity should be checked");
    let absent = limiter
        .acquire("password-reset\0missing@example.test", policy())
        .await
        .expect("absent identity should be checked");

    assert!(matches!(present, Permit::Allowed { .. }));
    assert!(matches!(absent, Permit::Allowed { .. }));

    let present_again = limiter
        .acquire("password-reset\0known@example.test", policy())
        .await
        .expect("present identity should remain rate limited");
    let absent_again = limiter
        .acquire("password-reset\0missing@example.test", policy())
        .await
        .expect("absent identity should remain rate limited");
    assert!(matches!(present_again, Permit::Rejected { .. }));
    assert!(matches!(absent_again, Permit::Rejected { .. }));
}

#[tokio::test]
async fn backend_errors_fail_closed() {
    let limiter = DeterministicLimiter::failing();
    let outcome = limiter
        .acquire("oauth-begin\0opaque-identity", policy())
        .await;

    assert!(matches!(outcome, Err(Error::DependencyUnavailable { .. })));
    assert!(!matches!(outcome, Ok(Permit::Allowed { .. })));
}

#[cfg(feature = "redis")]
#[test]
fn redis_keys_hash_route_and_normalized_identity_without_raw_values() {
    use magnetar::drivers::redis_abuse::RedisAbuseLimiter;

    let first = RedisAbuseLimiter::<()>::redis_key_for(" Password-Reset ", "  User@Example.TEST ");
    let equivalent = RedisAbuseLimiter::<()>::redis_key_for("password-reset", "user@example.test");
    let other_route = RedisAbuseLimiter::<()>::redis_key_for("oauth-begin", "user@example.test");

    assert_eq!(first, equivalent);
    assert_ne!(first, other_route);
    assert!(!first.contains("User@Example.TEST"));
    assert!(!first.contains("user@example.test"));
}

#[cfg(feature = "redis")]
mod redis_driver_tests {
    use super::*;
    use magnetar::drivers::redis_abuse::{
        RedisAbuseConnection, RedisAbuseLimiter, RedisPermitState,
    };

    struct FakeRedis {
        counts: Mutex<HashMap<String, u64>>,
        backend_error: bool,
    }

    #[async_trait]
    impl RedisAbuseConnection for FakeRedis {
        async fn increment_window(&self, key: &str, window: Duration) -> Result<RedisPermitState> {
            if self.backend_error {
                return Err(Error::DependencyUnavailable {
                    dependency: "fake-redis".to_owned(),
                    message: "deterministic backend failure".to_owned(),
                });
            }
            let mut counts = self.counts.lock().expect("test lock must not be poisoned");
            let count = counts.entry(key.to_owned()).or_default();
            *count += 1;
            Ok(RedisPermitState {
                count: *count,
                remaining: Some(window),
            })
        }
    }

    fn limiter() -> RedisAbuseLimiter<FakeRedis> {
        RedisAbuseLimiter::new(FakeRedis {
            counts: Mutex::new(HashMap::new()),
            backend_error: false,
        })
    }

    #[tokio::test]
    async fn redis_driver_enforces_allow_reject_boundary() {
        let limiter = limiter();
        let policy = AbusePolicy {
            max_requests: 2,
            window: Duration::from_secs(60),
        };
        assert!(matches!(
            limiter
                .acquire_identity("login", "USER@example.test", policy)
                .await,
            Ok(Permit::Allowed { .. })
        ));
        assert!(matches!(
            limiter
                .acquire_identity("login", "user@example.test", policy)
                .await,
            Ok(Permit::Allowed { .. })
        ));
        assert!(matches!(
            limiter
                .acquire_identity("login", "user@example.test", policy)
                .await,
            Ok(Permit::Rejected { .. })
        ));
    }

    #[tokio::test]
    async fn redis_driver_validates_policy_before_backend_access() {
        let limiter = limiter();
        let invalid = AbusePolicy {
            max_requests: 0,
            window: Duration::from_secs(60),
        };
        assert!(matches!(
            limiter
                .acquire_identity("login", "user@example.test", invalid)
                .await,
            Err(Error::InvalidInput { .. })
        ));
    }

    #[tokio::test]
    async fn redis_driver_propagates_backend_error_fail_closed() {
        let limiter = RedisAbuseLimiter::new(FakeRedis {
            counts: Mutex::new(HashMap::new()),
            backend_error: true,
        });
        let outcome = limiter
            .acquire_identity("login", "user@example.test", policy())
            .await;
        assert!(matches!(outcome, Err(Error::DependencyUnavailable { .. })));
    }

    #[tokio::test]
    async fn direct_route_scoped_key_shares_identity_counter() {
        let limiter = limiter();
        let policy = AbusePolicy {
            max_requests: 1,
            window: Duration::from_secs(60),
        };
        assert!(matches!(
            limiter
                .acquire_identity(" login ", " user@example.test ", policy)
                .await,
            Ok(Permit::Allowed { .. })
        ));
        assert!(matches!(
            limiter.acquire("login\0user@example.test", policy).await,
            Ok(Permit::Rejected { .. })
        ));
    }
}
