//! Shared Redis-backed abuse limiting.
//!
//! This module contains no framework or request types. Route and identity
//! values are normalized and SHA-256 hashed before they reach Redis, so raw
//! email addresses and bearer values are never Redis key material.

use std::time::Duration;

use async_trait::async_trait;
use redis::{Script, aio::MultiplexedConnection};
use sha2::{Digest, Sha256};

use crate::{
    Error, Result,
    abuse::{AbuseLimiter, AbusePolicy, Permit},
};

const KEY_PREFIX: &str = "magnetar:abuse:v1:";
const WINDOW_SCRIPT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
local remaining = redis.call('PTTL', KEYS[1])
return {count, remaining}
"#;

/// The backend result returned after one atomic Redis window increment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedisPermitState {
    /// Number of requests observed in the current window.
    pub count: u64,
    /// Remaining window duration, if Redis reported one.
    pub remaining: Option<Duration>,
}

/// A framework-neutral asynchronous Redis command boundary.
#[async_trait]
pub trait RedisAbuseConnection: Send + Sync {
    /// Atomically increment a key and return its count and remaining window.
    async fn increment_window(&self, key: &str, window: Duration) -> Result<RedisPermitState>;
}

/// An abuse limiter backed by a shared Redis connection boundary.
///
/// The generic connection is deliberately a trait rather than a framework
/// client. Production callers should construct this with [`RedisConnection`]
/// or another shared Redis implementation; there is no in-process fallback.
pub struct RedisAbuseLimiter<C> {
    connection: C,
}

impl<C> RedisAbuseLimiter<C> {
    /// Construct a limiter from a shared Redis connection boundary.
    #[must_use]
    pub const fn new(connection: C) -> Self {
        Self { connection }
    }

    /// Return the opaque Redis key for a route purpose and identity.
    ///
    /// The route purpose is included in the hash domain, while both it and the
    /// normalized identity remain absent from the returned key.
    #[must_use]
    pub fn redis_key_for(route_purpose: &str, identity: &str) -> String {
        let scoped_key = format!(
            "{}\0{}",
            Self::normalize_identity(route_purpose),
            Self::normalize_identity(identity)
        );
        Self::redis_key_for_scoped(&scoped_key)
    }

    fn redis_key_for_scoped(scoped_key: &str) -> String {
        let mut components = scoped_key.splitn(2, '\0');
        let route_purpose = Self::normalize_identity(components.next().unwrap_or_default());
        let identity = Self::normalize_identity(components.next().unwrap_or_default());
        let normalized_scoped_key = format!("{route_purpose}\0{identity}");
        let mut hasher = Sha256::new();
        hasher.update(b"magnetar-abuse-scoped-v1\0");
        hasher.update(normalized_scoped_key.as_bytes());
        format!("{KEY_PREFIX}{:x}", hasher.finalize())
    }

    /// Return a normalized identity without exposing its original spelling to
    /// the backend key builder.
    #[must_use]
    pub fn normalize_identity(identity: &str) -> String {
        identity.trim().to_lowercase()
    }

    /// Consume a budget for an explicit route purpose and identity.
    pub async fn acquire_identity(
        &self,
        route_purpose: &str,
        identity: &str,
        policy: AbusePolicy,
    ) -> Result<Permit>
    where
        C: RedisAbuseConnection,
    {
        let scoped_key = format!("{route_purpose}\0{identity}");
        self.acquire_opaque_key(&Self::redis_key_for_scoped(&scoped_key), policy)
            .await
    }

    async fn acquire_opaque_key(&self, key: &str, policy: AbusePolicy) -> Result<Permit>
    where
        C: RedisAbuseConnection,
    {
        policy.validate()?;
        let state = self.connection.increment_window(key, policy.window).await?;
        let remaining = state.remaining.unwrap_or(policy.window);
        if state.count <= u64::from(policy.max_requests) {
            return Ok(Permit::Allowed {
                retry_after: Some(remaining),
            });
        }
        Ok(Permit::Rejected {
            retry_after: remaining,
        })
    }
}

#[async_trait]
impl<C> AbuseLimiter for RedisAbuseLimiter<C>
where
    C: RedisAbuseConnection,
{
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit> {
        // The generic contract receives a route-scoped key. Deriving its
        // opaque Redis key here keeps this entry point equivalent to
        // `acquire_identity` while ensuring raw identities never reach Redis.
        let hashed_key = Self::redis_key_for_scoped(key);
        self.acquire_opaque_key(&hashed_key, policy).await
    }
}

/// A concrete shared Redis connection implementation.
#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisConnection {
    connection: MultiplexedConnection,
}

#[cfg(feature = "redis")]
impl RedisConnection {
    /// Wrap a shared, multiplexed Redis connection.
    #[must_use]
    pub const fn new(connection: MultiplexedConnection) -> Self {
        Self { connection }
    }
    /// Open a multiplexed connection using a Redis client.
    pub async fn connect(client: &redis::Client) -> Result<Self> {
        client
            .get_multiplexed_async_connection()
            .await
            .map(Self::new)
            .map_err(|_| dependency_unavailable())
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl RedisAbuseConnection for RedisConnection {
    async fn increment_window(&self, key: &str, window: Duration) -> Result<RedisPermitState> {
        let milliseconds = window.as_millis().clamp(1, i64::MAX as u128) as i64;
        let mut connection = self.connection.clone();
        let (count, remaining): (i64, i64) = Script::new(WINDOW_SCRIPT)
            .key(key)
            .arg(milliseconds)
            .invoke_async(&mut connection)
            .await
            .map_err(|_| dependency_unavailable())?;
        if count < 0 {
            return Err(dependency_unavailable());
        }
        Ok(RedisPermitState {
            count: count as u64,
            remaining: (remaining > 0).then(|| Duration::from_millis(remaining as u64)),
        })
    }
}

fn dependency_unavailable() -> Error {
    Error::DependencyUnavailable {
        dependency: "redis".to_owned(),
        message: "shared abuse-limiter backend failed".to_owned(),
    }
}
