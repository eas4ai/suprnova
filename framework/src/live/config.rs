//! Validated application configuration for Suprnova Live.

use std::error::Error;
use std::fmt;

use crate::FrameworkError;
use crate::render_cache::providers::redis::REDACTED_URL;

const HARD_MAX_CONTROL_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_CONTROL_BYTES: usize = 1024 * 1024;
const HARD_MAX_CONTEXT_LIFETIME_MS: u64 = 300_000;
const DEFAULT_CONTEXT_LIFETIME_MS: u64 = 30_000;

/// Where the Live instance ledger keeps its records.
///
/// The instance ledger is revision authority: it decides which mounted
/// component instance may accept the next action, and at most one accepted
/// outcome exists per base revision. A deployment of more than one node has
/// to keep that authority somewhere every node reads, which is what the two
/// distributed drivers are for. [`Self::Memory`] is the default and remains
/// correct for exactly one process.
///
/// The bounds a ledger runs under (instance lifetime, retained outcomes,
/// capacity) are the same constants in every driver: what changes is where
/// the records live, never what the state machine over them permits.
#[derive(Clone, Eq, PartialEq)]
pub enum LedgerDriver {
    /// One process, its own records. Records are lost with the process,
    /// which for a single node is the same thing as the process's own
    /// instances being gone.
    Memory,
    /// Records in the two Live record tables the RenderCache tier migration
    /// creates, shared by every node pointed at the same database, and
    /// coupled to the host transaction when one is open.
    Database,
    /// Records in Redis, shared by every node pointed at the same instance
    /// and prefix. Redis cannot join a host transaction, so a claim there is
    /// made before the transaction it belongs to commits - which spec 05
    /// permits, and which is the difference an operator is choosing between
    /// this driver and [`Self::Database`].
    Redis {
        /// Where the records are kept. It can carry a password, so it is
        /// never printed: see this type's [`fmt::Debug`] implementation.
        url: String,
        /// The literal string every key this deployment writes begins with.
        prefix: String,
    },
}

/// Manual, not derived: the [`LedgerDriver::Redis`] endpoint can carry a
/// password, and a driver reaches boot errors and logs.
impl fmt::Debug for LedgerDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory => formatter.write_str("Memory"),
            Self::Database => formatter.write_str("Database"),
            Self::Redis { url: _, prefix } => formatter
                .debug_struct("Redis")
                .field("url", &REDACTED_URL)
                .field("prefix", prefix)
                .finish(),
        }
    }
}

impl LedgerDriver {
    /// Reads `LIVE_LEDGER_DRIVER` (`memory` default, `database`, `redis`),
    /// with `LIVE_REDIS_URL` (falling back to `REDIS_URL`, then to
    /// `redis://127.0.0.1:6379`) and `LIVE_REDIS_PREFIX` for the Redis
    /// driver.
    ///
    /// The prefix defaults to this crate's own Live key namespace, spelled
    /// out in the `DEFAULT_LIVE_REDIS_PREFIX` constant beside this function
    /// (crate-private, so this is a plain code span rather than a link, and
    /// the literal is not repeated here: the rendered public Live
    /// documentation must not carry an internal crate path, and this
    /// namespace is that path plus a colon).
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `LIVE_LEDGER_DRIVER` names a driver
    /// this build does not have. The message names the variable and never
    /// repeats the rejected value: an environment value can carry a secret.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_source(&|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] over any reader, so the parser can be proven
    /// against fixed pairs rather than against the process environment.
    fn from_source(read: &dyn Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let non_empty = |name: &str| read(name).filter(|value| !value.is_empty());
        match non_empty("LIVE_LEDGER_DRIVER").as_deref() {
            None | Some("memory") => Ok(Self::Memory),
            Some("database") => Ok(Self::Database),
            Some("redis") => Ok(Self::Redis {
                url: non_empty("LIVE_REDIS_URL")
                    .or_else(|| non_empty("REDIS_URL"))
                    .unwrap_or_else(|| DEFAULT_LIVE_REDIS_URL.to_owned()),
                prefix: non_empty("LIVE_REDIS_PREFIX")
                    .unwrap_or_else(|| DEFAULT_LIVE_REDIS_PREFIX.to_owned()),
            }),
            Some(_) => Err(FrameworkError::internal(
                "LiveRuntime: LIVE_LEDGER_DRIVER is set to a value that is not one of memory, \
                 database, or redis. The rejected value is not repeated here: an environment \
                 value can carry a secret. Set LIVE_LEDGER_DRIVER to one of the accepted \
                 values, or unset it to run the in-process ledger.",
            )),
        }
    }
}

/// Where the Redis ledger driver connects when neither `LIVE_REDIS_URL` nor
/// `REDIS_URL` names an endpoint.
const DEFAULT_LIVE_REDIS_URL: &str = "redis://127.0.0.1:6379";
/// The key namespace the Redis ledger driver writes under when
/// `LIVE_REDIS_PREFIX` does not name one.
const DEFAULT_LIVE_REDIS_PREFIX: &str = "suprnova_live:";

/// Validated byte limits applied to one Live control request and response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfig {
    max_request_bytes: usize,
    max_response_bytes: usize,
    max_context_lifetime_ms: u64,
}

impl LiveConfig {
    /// Starts a builder with conservative one-megabyte control-envelope limits.
    #[must_use]
    pub const fn builder() -> LiveConfigBuilder {
        LiveConfigBuilder::new()
    }

    /// Returns conservative defaults suitable for an ordinary application.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: DEFAULT_CONTROL_BYTES,
            max_context_lifetime_ms: DEFAULT_CONTEXT_LIFETIME_MS,
        }
    }

    /// Returns the maximum accepted bytes for one complete Live request body.
    #[must_use]
    pub const fn max_request_bytes(self) -> usize {
        self.max_request_bytes
    }

    /// Returns the maximum emitted bytes for one complete Live response body.
    #[must_use]
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }

    /// Returns the maximum validity window for one trusted request context.
    #[must_use]
    pub const fn max_context_lifetime_ms(self) -> u64 {
        self.max_context_lifetime_ms
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self::standard()
    }
}

/// Startup-only builder for [`LiveConfig`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveConfigBuilder {
    max_request_bytes: usize,
    max_response_bytes: usize,
    max_context_lifetime_ms: u64,
}

impl LiveConfigBuilder {
    /// Creates a builder with [`LiveConfig::standard`] byte limits.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_request_bytes: DEFAULT_CONTROL_BYTES,
            max_response_bytes: DEFAULT_CONTROL_BYTES,
            max_context_lifetime_ms: DEFAULT_CONTEXT_LIFETIME_MS,
        }
    }

    /// Sets the whole-request body ceiling.
    #[must_use]
    pub const fn max_request_bytes(mut self, max: usize) -> Self {
        self.max_request_bytes = max;
        self
    }

    /// Sets the complete encoded response-body ceiling.
    #[must_use]
    pub const fn max_response_bytes(mut self, max: usize) -> Self {
        self.max_response_bytes = max;
        self
    }

    /// Sets the maximum validity window for one trusted request context.
    #[must_use]
    pub const fn max_context_lifetime_ms(mut self, max: u64) -> Self {
        self.max_context_lifetime_ms = max;
        self
    }

    /// Validates the configured limits and creates immutable Live configuration.
    pub fn build(self) -> Result<LiveConfig, LiveConfigError> {
        let valid = (1..=HARD_MAX_CONTROL_BYTES).contains(&self.max_request_bytes)
            && (1..=HARD_MAX_CONTROL_BYTES).contains(&self.max_response_bytes)
            && self.max_response_bytes <= self.max_request_bytes;
        if !valid {
            return Err(LiveConfigError::new(LiveConfigErrorKind::InvalidByteLimits));
        }
        if !(1..=HARD_MAX_CONTEXT_LIFETIME_MS).contains(&self.max_context_lifetime_ms) {
            return Err(LiveConfigError::new(
                LiveConfigErrorKind::InvalidContextLifetime,
            ));
        }
        Ok(LiveConfig {
            max_request_bytes: self.max_request_bytes,
            max_response_bytes: self.max_response_bytes,
            max_context_lifetime_ms: self.max_context_lifetime_ms,
        })
    }
}

impl Default for LiveConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Closed reason Live startup configuration was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LiveConfigErrorKind {
    /// A request or response ceiling was zero, above the hard ceiling, or inconsistent.
    InvalidByteLimits,
    /// The trusted request-context lifetime was zero or exceeded the engine ceiling.
    InvalidContextLifetime,
}

impl LiveConfigErrorKind {
    /// Returns the stable machine-readable failure value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidByteLimits => "invalid_live_byte_limits",
            Self::InvalidContextLifetime => "invalid_live_context_lifetime",
        }
    }
}

/// Redacted Live configuration failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LiveConfigError {
    kind: LiveConfigErrorKind,
}

impl LiveConfigError {
    const fn new(kind: LiveConfigErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed configuration failure category.
    #[must_use]
    pub const fn kind(self) -> LiveConfigErrorKind {
        self.kind
    }
}

impl fmt::Display for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for LiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LiveConfigError {}

#[cfg(test)]
mod ledger_driver_tests {
    //! Which instance ledger a deployment runs, and what a rejected value
    //! may say.
    //!
    //! Every test reads a table rather than the process environment, for the
    //! reason `render_cache::config`'s own tests do: the environment is
    //! shared by every test in a binary and a table is not.
    use super::*;

    fn source(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn parsed(pairs: &[(&str, &str)]) -> LedgerDriver {
        let table = source(pairs);
        LedgerDriver::from_source(&|name| table.get(name).cloned())
            .expect("the fixture configuration parses")
    }

    #[test]
    fn the_memory_ledger_is_the_default_and_the_only_one_needing_nothing() {
        assert_eq!(parsed(&[]), LedgerDriver::Memory);
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "memory")]),
            LedgerDriver::Memory
        );
    }

    #[test]
    fn the_distributed_drivers_carry_what_they_need_to_reach_their_store() {
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "database")]),
            LedgerDriver::Database
        );
        assert_eq!(
            parsed(&[("LIVE_LEDGER_DRIVER", "redis")]),
            LedgerDriver::Redis {
                url: "redis://127.0.0.1:6379".to_owned(),
                prefix: "suprnova_live:".to_owned(),
            }
        );
        // The shared endpoint, then Live's own, then the prefix.
        assert_eq!(
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("REDIS_URL", "redis://shared:6379"),
            ]),
            LedgerDriver::Redis {
                url: "redis://shared:6379".to_owned(),
                prefix: "suprnova_live:".to_owned(),
            }
        );
        assert_eq!(
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("REDIS_URL", "redis://shared:6379"),
                ("LIVE_REDIS_URL", "redis://live:6379"),
                ("LIVE_REDIS_PREFIX", "tenant_a_live:"),
            ]),
            LedgerDriver::Redis {
                url: "redis://live:6379".to_owned(),
                prefix: "tenant_a_live:".to_owned(),
            }
        );
    }

    #[test]
    fn an_unknown_driver_names_the_variable_and_repeats_nothing() {
        let table = source(&[("LIVE_LEDGER_DRIVER", "cassandra")]);
        let message = LedgerDriver::from_source(&|name| table.get(name).cloned())
            .expect_err("an unknown driver is refused")
            .to_string();
        assert!(message.contains("LIVE_LEDGER_DRIVER"), "{message}");
        assert!(!message.contains("cassandra"), "{message}");
    }

    #[test]
    fn a_live_redis_endpoint_never_reaches_a_printed_driver() {
        let printed = format!(
            "{:?}",
            parsed(&[
                ("LIVE_LEDGER_DRIVER", "redis"),
                ("LIVE_REDIS_URL", "redis://someone:hunter2@10.0.0.1:6379"),
            ])
        );
        assert!(printed.contains("redis://<redacted>"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
        assert!(!printed.contains("10.0.0.1"), "{printed}");
    }
}
