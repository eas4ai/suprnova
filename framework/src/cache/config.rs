//! Cache configuration for suprnova framework

use crate::config::{env, env_optional};
use crate::error::FrameworkError;
use crate::render_cache::providers::redis::REDACTED_URL;

/// Which cache backend to bootstrap.
///
/// Selected via the `CACHE_DRIVER` env var (parsed case-insensitively).
/// Defaults to [`CacheDriver::Memory`] when unset - single-process dev
/// loops don't need Redis, and the previous "try Redis, silently fall
/// back to memory on failure" behaviour was actively dangerous in
/// production. Operators choosing Redis MUST set the driver explicitly
/// so connection failures surface at boot instead of silently degrading
/// to per-process in-memory state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheDriver {
    /// Per-process in-memory cache. Default. No external dependencies.
    #[default]
    Memory,
    /// Redis-backed cache. Reads `REDIS_URL`. Boot fails closed if the
    /// configured URL is unreachable.
    Redis,
}

impl CacheDriver {
    /// Parse a `CACHE_DRIVER` env-var value. Case-insensitive; trims
    /// whitespace; returns an `internal` error for unknown driver names
    /// so misconfigurations surface at boot.
    pub fn parse(s: &str) -> Result<Self, FrameworkError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "memory" | "in-memory" | "inmemory" => Ok(Self::Memory),
            "redis" => Ok(Self::Redis),
            other => Err(FrameworkError::internal(format!(
                "CACHE_DRIVER: unknown driver `{other}` (expected `memory` or `redis`)"
            ))),
        }
    }
}

/// Cache configuration
///
/// # Environment Variables
///
/// - `CACHE_DRIVER` - `memory` (default) or `redis`. Selects the
///   bootstrap target. Memory keeps everything in this process; Redis
///   requires `REDIS_URL` and fails boot if unreachable.
/// - `REDIS_URL` - Redis connection URL (default: redis://127.0.0.1:6379)
/// - `REDIS_PREFIX` - Key prefix for cache entries (default: "suprnova_cache:")
/// - `CACHE_DEFAULT_TTL` - Default TTL in seconds, 0 = no expiration (default: 3600)
/// - `CACHE_SWEEP_INTERVAL` - Seconds between sweeps of the in-memory
///   driver's expired entries, 0 = no sweep (default: 60)
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::{Config, CacheConfig};
/// use suprnova::cache::CacheDriver;
/// # fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// // Register from environment
/// Config::register(CacheConfig::from_env()?);
///
/// // Or build manually
/// Config::register(CacheConfig::builder()
///     .driver(CacheDriver::Redis)
///     .url("redis://localhost:6379")
///     .prefix("myapp:")
///     .build());
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct CacheConfig {
    /// Which backend to bootstrap. Defaults to in-memory.
    pub driver: CacheDriver,
    /// Redis connection URL (consulted only when `driver == Redis`). It
    /// can carry a password, so it is never printed: see this type's
    /// `Debug` implementation.
    pub url: String,
    /// Key prefix for all cache entries
    pub prefix: String,
    /// Default TTL in seconds (0 = no expiration). The facade applies
    /// this to `Cache::put(None)`, `Cache::remember(None)` and
    /// `Cache::tags_put(None)`. `Cache::forever`, `Cache::remember_forever`
    /// and `Cache::tags_forever` always bypass it.
    pub default_ttl: u64,
    /// Seconds between sweeps of the in-memory driver's expired entries
    /// (0 = no sweep). A read removes the expired entry it finds, but a
    /// key that expires and is never read again stays in memory until a
    /// sweep removes it. Redis expires keys itself and ignores this.
    pub sweep_interval: u64,
}

impl std::fmt::Debug for CacheConfig {
    /// Prints everything but the Redis URL, which routinely carries a
    /// password and reaches logs through whatever prints the configuration.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheConfig")
            .field("driver", &self.driver)
            .field("url", &REDACTED_URL)
            .field("prefix", &self.prefix)
            .field("default_ttl", &self.default_ttl)
            .field("sweep_interval", &self.sweep_interval)
            .finish()
    }
}

/// Where the Redis URL `url` connects - `host:port`, or a socket path -
/// without its credentials, for messages that have to say where.
///
/// The URL is parsed the way the Redis client parses it, so the endpoint
/// named is the one a connection was attempted against. A URL the client
/// cannot parse is named as such rather than repeated: it may still hold a
/// password.
pub(crate) fn redis_endpoint(url: &str) -> String {
    match redis::IntoConnectionInfo::into_connection_info(url) {
        Ok(info) => info.addr().to_string(),
        Err(_) => "a REDIS_URL the Redis client cannot parse".to_string(),
    }
}

impl CacheConfig {
    /// Create configuration from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an internal error if `CACHE_DRIVER` is set to an unknown
    /// value. Unset means "use the default", which is in-memory.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let driver = match env_optional::<String>("CACHE_DRIVER") {
            Some(s) => CacheDriver::parse(&s)?,
            None => CacheDriver::default(),
        };
        Ok(Self {
            driver,
            url: env_optional("REDIS_URL").unwrap_or_else(|| "redis://127.0.0.1:6379".to_string()),
            prefix: env("REDIS_PREFIX", "suprnova_cache:".to_string()),
            default_ttl: env("CACHE_DEFAULT_TTL", 3600),
            sweep_interval: env("CACHE_SWEEP_INTERVAL", 60),
        })
    }

    /// Create a builder for manual configuration
    pub fn builder() -> CacheConfigBuilder {
        CacheConfigBuilder::default()
    }
}

impl Default for CacheConfig {
    /// In-memory driver, framework default URL/prefix/TTL - designed to
    /// succeed without env vars set. Use `CacheConfig::from_env()` (which
    /// returns a `Result`) when the caller wants `CACHE_DRIVER` parsing
    /// errors to surface.
    fn default() -> Self {
        Self {
            driver: CacheDriver::Memory,
            url: "redis://127.0.0.1:6379".to_string(),
            prefix: "suprnova_cache:".to_string(),
            default_ttl: 3600,
            sweep_interval: 60,
        }
    }
}

/// Builder for CacheConfig
#[derive(Default)]
pub struct CacheConfigBuilder {
    driver: Option<CacheDriver>,
    url: Option<String>,
    prefix: Option<String>,
    default_ttl: Option<u64>,
    sweep_interval: Option<u64>,
}

impl std::fmt::Debug for CacheConfigBuilder {
    /// Prints everything but the Redis URL, for the reason
    /// [`CacheConfig`]'s `Debug` gives.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheConfigBuilder")
            .field("driver", &self.driver)
            .field("url", &self.url.as_ref().map(|_| REDACTED_URL))
            .field("prefix", &self.prefix)
            .field("default_ttl", &self.default_ttl)
            .field("sweep_interval", &self.sweep_interval)
            .finish()
    }
}

impl CacheConfigBuilder {
    /// Pick the backend explicitly.
    pub fn driver(mut self, driver: CacheDriver) -> Self {
        self.driver = Some(driver);
        self
    }

    /// Set the Redis URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the key prefix
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Set the default TTL in seconds
    pub fn default_ttl(mut self, seconds: u64) -> Self {
        self.default_ttl = Some(seconds);
        self
    }

    /// Set the seconds between sweeps of the in-memory driver's expired
    /// entries; 0 turns the sweep off.
    pub fn sweep_interval(mut self, seconds: u64) -> Self {
        self.sweep_interval = Some(seconds);
        self
    }

    /// Build the configuration. Falls back to `CacheConfig::default()`
    /// for any unset field rather than re-reading the environment, so
    /// the builder is fully deterministic.
    pub fn build(self) -> CacheConfig {
        let defaults = CacheConfig::default();
        CacheConfig {
            driver: self.driver.unwrap_or(defaults.driver),
            url: self.url.unwrap_or(defaults.url),
            prefix: self.prefix.unwrap_or(defaults.prefix),
            default_ttl: self.default_ttl.unwrap_or(defaults.default_ttl),
            sweep_interval: self.sweep_interval.unwrap_or(defaults.sweep_interval),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Debug` output of the cache configuration never carries the
    /// credentials in its Redis URL, in the userinfo or in the `pass` query
    /// parameter a socket URL takes. It used to print the URL whole, so any
    /// log line that formatted the configuration leaked the password.
    #[test]
    fn debug_output_never_carries_redis_credentials() {
        for url in [
            "redis://cache-user:s3cret-pw@cache.internal:6380/2",
            "redis+unix:///run/redis.sock?user=cache-user&pass=s3cret-pw",
        ] {
            let built = CacheConfig::builder().driver(CacheDriver::Redis).url(url);
            let builder_debug = format!("{built:?}");
            let config_debug = format!("{:?}", built.build());
            for rendered in [config_debug, builder_debug] {
                assert!(
                    !rendered.contains("s3cret-pw") && !rendered.contains("cache-user"),
                    "Debug leaks REDIS_URL credentials: {rendered}"
                );
            }
        }
    }
}
