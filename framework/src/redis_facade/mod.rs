//! The `Redis` facade: named connections to Redis servers, for the
//! commands the cache, the queue and the rate limiter do not run for you.
//!
//! The `default` connection reaches `REDIS_URL`; the application's
//! bootstrap names others with [`Redis::define`]. Connections are resolved
//! by name, opened on their first command, and kept until
//! [`Redis::purge`].

mod connection;
mod events;
mod pipeline;
mod subscription;
mod value;

pub use connection::{RedisConnection, RedisSide};
pub use events::{RedisCommandExecuted, RedisCommandFailed};
pub use pipeline::RedisPipeline;
pub use subscription::{RedisMessage, RedisSubscription};
pub use value::RedisValue;

use crate::error::FrameworkError;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Where the `default` connection goes when `REDIS_URL` is unset.
const DEFAULT_URL: &str = "redis://127.0.0.1:6379";

/// The connections the bootstrap defined, and the ones resolved.
struct Registry {
    defined: BTreeMap<String, redis::Client>,
    resolved: BTreeMap<String, RedisConnection>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    defined: BTreeMap::new(),
    resolved: BTreeMap::new(),
});

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A client for `url`, which must be a Redis URL: `redis://`, `rediss://`,
/// `redis+unix://` or `unix://`. `what` names the connection in the error;
/// the URL is never echoed, since it may hold a password.
fn client_for(url: &str, what: &str) -> Result<redis::Client, FrameworkError> {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme);
    if !matches!(
        scheme,
        Some("redis" | "rediss" | "redis+unix" | "unix" | "valkey" | "valkeys" | "valkey+unix")
    ) {
        return Err(FrameworkError::internal(format!(
            "{what}: the URL is not a Redis URL (redis:// or unix://, or rediss:// with a TLS \
             feature of the redis crate)"
        )));
    }
    crate::redis_client::open(url).map_err(|error| {
        FrameworkError::from_external_with(format!("{what}: the URL is not usable: {error}"), error)
    })
}

/// The facade over named Redis connections.
///
/// ```no_run
/// use suprnova::Redis;
///
/// # async fn example() -> Result<(), suprnova::FrameworkError> {
/// let redis = Redis::connection("default")?;
/// redis.set("greeting", "hello").await?;
/// let greeting = redis.get("greeting").await?;
/// # Ok(())
/// # }
/// ```
pub struct Redis;

impl Redis {
    /// Give the connection `name` the server at `url`, replacing any
    /// connection of that name, `default` included. Call it in the
    /// bootstrap. A name already resolved stays resolved, on the new URL:
    /// the next [`connection`](Self::connection) reaches it, while handles
    /// taken before keep the server they had.
    ///
    /// # Errors
    ///
    /// When `url` is not a Redis URL.
    pub fn define(name: &str, url: &str) -> Result<(), FrameworkError> {
        let client = client_for(url, &format!("the Redis connection '{name}'"))?;
        Self::define_client(name, client);
        Ok(())
    }

    /// Give the connection `name` a client you built yourself, for what a
    /// URL cannot say: TLS with certificates of your own, or a server a
    /// Sentinel names. Otherwise as [`define`](Self::define).
    pub fn define_client(name: &str, client: redis::Client) {
        crate::redis_client::ensure_crypto_provider();
        let mut registry = registry();
        if registry.resolved.contains_key(name) {
            let replaced = RedisConnection::new(name, client.clone());
            registry.resolved.insert(name.to_owned(), replaced);
        }
        registry.defined.insert(name.to_owned(), client);
    }

    /// The connection named `name`. Resolving it sends nothing: the
    /// connection opens on its first command.
    ///
    /// # Errors
    ///
    /// When no connection has the name, or, for `default`, when `REDIS_URL`
    /// is not a Redis URL.
    pub fn connection(name: &str) -> Result<RedisConnection, FrameworkError> {
        let mut registry = registry();
        if let Some(connection) = registry.resolved.get(name) {
            return Ok(connection.clone());
        }
        let client = match registry.defined.get(name) {
            Some(client) => client.clone(),
            None if name == "default" => {
                let url = std::env::var("REDIS_URL")
                    .ok()
                    .filter(|url| !url.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_URL.to_owned());
                client_for(&url, "the default Redis connection's REDIS_URL")?
            }
            None => {
                return Err(FrameworkError::internal(format!(
                    "no Redis connection is named '{name}': define it with Redis::define in the \
                     bootstrap"
                )));
            }
        };
        let connection = RedisConnection::new(name, client);
        registry
            .resolved
            .insert(name.to_owned(), connection.clone());
        Ok(connection)
    }

    /// The `default` connection.
    ///
    /// # Errors
    ///
    /// As [`connection`](Self::connection).
    pub fn default_connection() -> Result<RedisConnection, FrameworkError> {
        Self::connection("default")
    }

    /// Forget the resolved connection `name`. It closes once no handle
    /// holds it, and the next [`connection`](Self::connection) opens a new
    /// one. A defined connection stays defined.
    pub fn purge(name: &str) {
        registry().resolved.remove(name);
    }

    /// The names of the connections resolved and not purged.
    pub fn connections() -> Vec<String> {
        registry().resolved.keys().cloned().collect()
    }

    /// Report every command a connection runs, outside a pipeline or a
    /// transaction, to the listeners [`listen`](Self::listen) and
    /// [`listen_for_failures`](Self::listen_for_failures) add. Off until
    /// called.
    pub fn enable_events() {
        events::enable(true);
    }

    /// Stop reporting commands.
    pub fn disable_events() {
        events::enable(false);
    }

    /// Add a listener for each command that ran, while events are enabled.
    /// It runs on the task that sent the command, so it should be quick.
    pub fn listen(listener: impl Fn(&RedisCommandExecuted) + Send + Sync + 'static) {
        events::listen(Arc::new(listener));
    }

    /// Add a listener for each command that failed, while events are
    /// enabled.
    pub fn listen_for_failures(listener: impl Fn(&RedisCommandFailed) + Send + Sync + 'static) {
        events::listen_for_failures(Arc::new(listener));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_redis_schemes_are_redis_urls() {
        assert!(client_for("redis://127.0.0.1:6379/2", "x").is_ok());
        assert!(client_for("unix:///tmp/redis.sock", "x").is_ok());
        let error = client_for("http://user:secret@x", "the connection 'x'").unwrap_err();
        assert!(error.to_string().contains("the connection 'x'"));
        assert!(
            !error.to_string().contains("secret"),
            "the URL is never echoed"
        );
        assert!(client_for("127.0.0.1:6379", "x").is_err());
    }

    #[test]
    fn resolving_needs_no_runtime_and_sends_nothing() {
        Redis::define("unit-nowhere", "redis://127.0.0.1:1/0").unwrap();
        let connection = Redis::connection("unit-nowhere").unwrap();
        assert_eq!(connection.name(), "unit-nowhere");
        assert!(Redis::connections().contains(&"unit-nowhere".to_owned()));
        assert!(
            connection.client().is_err(),
            "outside a runtime the client cannot run"
        );
        Redis::purge("unit-nowhere");
        assert!(!Redis::connections().contains(&"unit-nowhere".to_owned()));
    }
}
