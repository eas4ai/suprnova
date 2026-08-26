//! Database configuration for suprnova framework

use crate::FrameworkError;
use crate::config::{Environment, env, env_optional};

/// Database type enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseType {
    /// PostgreSQL - `postgres://` or `postgresql://` URL.
    Postgres,
    /// MySQL or MariaDB - `mysql://` URL.
    Mysql,
    /// SQLite - `sqlite://` URL, or an absolute file path.
    Sqlite,
    /// Unrecognized scheme; the database layer will refuse to connect.
    Unknown,
}

/// Source provenance of [`DatabaseConfig::url`].
///
/// Tracks whether the URL came from the `DATABASE_URL` env variable
/// (`Env`), was filled in by the silent SQLite fallback (`Default`),
/// or was supplied explicitly via [`DatabaseConfigBuilder::url`]
/// (`Explicit`). Audit HIGH `database` #1: production boots must
/// refuse the silent fallback - see
/// [`DatabaseConfig::validate_for_environment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlSource {
    /// URL was read from the `DATABASE_URL` env var.
    Env,
    /// URL fell through to the dev-convenience `sqlite://./database.db`
    /// fallback because `DATABASE_URL` was unset.
    Default,
    /// URL was set programmatically (typically via the builder).
    Explicit,
}

/// Database configuration
///
/// # Environment Variables
///
/// - `DATABASE_URL` - Full connection URL (required for connection, defaults to sqlite://./database.db)
/// - `DB_MAX_CONNECTIONS` - Maximum pool connections (default: 10)
/// - `DB_MIN_CONNECTIONS` - Minimum pool connections (default: 1)
/// - `DB_CONNECT_TIMEOUT` - Connection timeout in seconds (default: 30)
/// - `DB_LOGGING` - Enable SQL logging (default: false)
/// - `DB_IDLE_TIMEOUT` - Seconds before an idle pooled connection is closed (default: sqlx's 600; `0` disables)
/// - `DB_MAX_LIFETIME` - Seconds a pooled connection may live (default: sqlx's 1800; `0` disables)
/// - `DB_ACQUIRE_TIMEOUT` - Seconds to wait for a free connection (default: falls back to `DB_CONNECT_TIMEOUT`)
/// - `DB_TEST_BEFORE_ACQUIRE` - Ping a connection before handing it out (default: true)
/// - `DB_PING_AFTER_IDLE` - Ping only after this many idle seconds; setting it disables `DB_TEST_BEFORE_ACQUIRE` (default: unset)
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::{Config, DatabaseConfig};
///
/// # fn ex() {
/// // Register from environment
/// Config::register(DatabaseConfig::from_env());
///
/// // Or build manually
/// Config::register(DatabaseConfig::builder()
///     .url("postgres://localhost/mydb")
///     .max_connections(20)
///     .build());
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Full database connection URL
    pub url: String,
    /// Maximum connections in pool
    pub max_connections: u32,
    /// Minimum connections in pool
    pub min_connections: u32,
    /// Connection timeout in seconds
    pub connect_timeout: u64,
    /// Enable SQL query logging
    pub logging: bool,
    /// Seconds a pooled connection may sit idle before the pool closes
    /// it. `None` leaves sqlx's 600-second default in place; `Some(0)`
    /// means never reap on idleness.
    ///
    /// This is half the answer to a connection a NAT or firewall killed
    /// while nobody was using it: sqlx 0.9 exposes no libpq
    /// `keepalives_*` equivalent, so the socket cannot be kept warm and
    /// the pool has to stop trusting old connections instead.
    pub idle_timeout: Option<u64>,
    /// Maximum total lifetime of a pooled connection, in seconds.
    /// `None` leaves sqlx's 1800-second default; `Some(0)` means never
    /// recycle on age.
    ///
    /// Bounds the blast radius of anything that goes stale on a
    /// long-lived connection - a rotated credential, a failed-over
    /// replica, a middlebox that expires state on a schedule.
    pub max_lifetime: Option<u64>,
    /// Seconds to wait for a free pooled connection before erroring.
    /// `None` means the pool inherits [`Self::connect_timeout`], which
    /// is what SeaORM maps onto sqlx's `acquire_timeout` when this is
    /// unset. Setting it overrides that mapping.
    pub acquire_timeout: Option<u64>,
    /// Ping a pooled connection before handing it out. `true` is sqlx's
    /// default and the framework's existing behavior. Turn it off only
    /// when the per-checkout round trip is measurably too expensive and
    /// [`Self::ping_after_idle`] is not enough.
    pub test_before_acquire: bool,
    /// Ping a pooled connection only once it has been idle this many
    /// seconds, instead of on every checkout.
    ///
    /// Cheaper than [`Self::test_before_acquire`] under load: a hot
    /// connection is handed out untouched, and only a connection idle
    /// long enough to have plausibly been dropped pays for a round
    /// trip. Setting it forces `test_before_acquire` off - sqlx would
    /// otherwise run both hooks and ping on every acquire anyway.
    pub ping_after_idle: Option<u64>,
    /// Where [`Self::url`] came from - env var, dev-fallback default,
    /// or an explicit programmatic value. Used by
    /// [`Self::validate_for_environment`] to refuse the silent
    /// SQLite fallback in production.
    pub url_source: UrlSource,
}

impl DatabaseConfig {
    /// The dev-convenience SQLite fallback URL used when
    /// `DATABASE_URL` is unset. Public so production
    /// preflight tooling can compare against it without
    /// hard-coding the string.
    pub const DEFAULT_SQLITE_URL: &'static str = "sqlite://./database.db";

    /// Create configuration from environment variables.
    ///
    /// When `DATABASE_URL` is unset, falls back to
    /// [`Self::DEFAULT_SQLITE_URL`] and records the source as
    /// [`UrlSource::Default`] - that flag is what
    /// [`Self::validate_for_environment`] uses to refuse the silent
    /// fallback in production.
    pub fn from_env() -> Self {
        let (url, url_source) = match env_optional("DATABASE_URL") {
            Some(u) => (u, UrlSource::Env),
            None => (Self::DEFAULT_SQLITE_URL.to_string(), UrlSource::Default),
        };
        Self {
            url,
            max_connections: env("DB_MAX_CONNECTIONS", 10),
            min_connections: env("DB_MIN_CONNECTIONS", 1),
            connect_timeout: env("DB_CONNECT_TIMEOUT", 30),
            logging: env("DB_LOGGING", false),
            idle_timeout: env_optional("DB_IDLE_TIMEOUT"),
            max_lifetime: env_optional("DB_MAX_LIFETIME"),
            acquire_timeout: env_optional("DB_ACQUIRE_TIMEOUT"),
            test_before_acquire: env("DB_TEST_BEFORE_ACQUIRE", true),
            ping_after_idle: env_optional("DB_PING_AFTER_IDLE"),
            url_source,
        }
    }

    /// Create a builder for manual configuration
    pub fn builder() -> DatabaseConfigBuilder {
        DatabaseConfigBuilder::default()
    }

    /// Detect database type from URL
    pub fn database_type(&self) -> DatabaseType {
        if self.url.starts_with("postgres://") || self.url.starts_with("postgresql://") {
            DatabaseType::Postgres
        } else if self.url.starts_with("mysql://") {
            DatabaseType::Mysql
        } else if self.url.starts_with("sqlite://") || self.url.starts_with("sqlite:") {
            DatabaseType::Sqlite
        } else {
            DatabaseType::Unknown
        }
    }

    /// Returns whether the database URL was explicitly configured
    /// rather than falling through to the dev SQLite default.
    ///
    /// Use this as a precondition signal - production boots that
    /// observe `false` here must refuse to continue. The lower-level
    /// helper that gates `DB::init` on this is
    /// [`Self::validate_for_environment`].
    pub fn is_configured(&self) -> bool {
        self.url_source != UrlSource::Default
    }

    /// Refuse to boot in a production-like environment when the URL
    /// fell through to the dev SQLite fallback.
    ///
    /// "Production-like" = [`Environment::Production`] or
    /// [`Environment::Staging`]. Local / Development / Testing /
    /// Custom environments keep the silent fallback for zero-setup
    /// iteration, matching the project's documented dev posture
    /// ("Suprnova dev default = SQLite").
    ///
    /// Called automatically by [`DB::init`](crate::DB::init) and
    /// [`DB::init_with`](crate::DB::init_with); manual `DB::init_with`
    /// callers that pre-build a config can call this themselves to
    /// fail-fast at config-creation time if they want a tighter
    /// guarantee.
    pub fn validate_for_environment(&self, env: &Environment) -> Result<(), FrameworkError> {
        let prod_like = env.is_production() || matches!(env, Environment::Staging);
        if prod_like && self.url_source == UrlSource::Default {
            return Err(FrameworkError::param(format!(
                "DATABASE_URL is required in `{env}` but was unset - refusing to boot \
                 against the dev SQLite fallback `{}`. Set DATABASE_URL to the \
                 production database URL, or construct an explicit config via \
                 `DatabaseConfig::builder().url(...)` when a SQLite file really is \
                 the production database.",
                Self::DEFAULT_SQLITE_URL,
            )));
        }
        Ok(())
    }

    /// Refuse pool settings that would silently misbehave at runtime:
    ///
    /// - `max_connections == 0` - the pool will never hand out a
    ///   connection; every query times out.
    /// - `min_connections > max_connections` - sqlx accepts this but it
    ///   means the pool can never warm itself up.
    /// - `connect_timeout == 0` - the first call would error
    ///   immediately on any latency.
    /// - `acquire_timeout == Some(0)` - a zero-second pool checkout wait
    ///   fails every call the moment the pool is contended.
    ///
    /// `idle_timeout == Some(0)` and `max_lifetime == Some(0)` are
    /// deliberately left alone - `0` is the documented "never reap"
    /// spelling for those two knobs, not a misconfiguration.
    ///
    /// Called from [`DbConnection::connect`](crate::database::DbConnection)
    /// so both [`DB::init`](crate::DB::init) and
    /// [`DB::init_with`](crate::DB::init_with) fail-fast on a bad config
    /// instead of producing a sick pool. `min_connections == 0` is left
    /// alone - that's a legitimate "lazy / idle-empty pool" mode.
    pub fn validate_pool(&self) -> Result<(), FrameworkError> {
        if self.max_connections == 0 {
            return Err(FrameworkError::param(
                "DB_MAX_CONNECTIONS must be > 0; a zero-sized pool never hands out connections",
            ));
        }
        if self.min_connections > self.max_connections {
            return Err(FrameworkError::param(format!(
                "DB_MIN_CONNECTIONS ({}) must be <= DB_MAX_CONNECTIONS ({})",
                self.min_connections, self.max_connections,
            )));
        }
        if self.connect_timeout == 0 {
            return Err(FrameworkError::param(
                "DB_CONNECT_TIMEOUT must be > 0; a zero-second timeout fails immediately on any latency",
            ));
        }
        if self.acquire_timeout == Some(0) {
            return Err(FrameworkError::param(
                "DB_ACQUIRE_TIMEOUT must be > 0; a zero-second acquire timeout fails every \
                 checkout the moment the pool is contended",
            ));
        }
        Ok(())
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Builder for DatabaseConfig
#[derive(Debug, Default)]
pub struct DatabaseConfigBuilder {
    url: Option<String>,
    max_connections: Option<u32>,
    min_connections: Option<u32>,
    connect_timeout: Option<u64>,
    logging: Option<bool>,
    idle_timeout: Option<u64>,
    max_lifetime: Option<u64>,
    acquire_timeout: Option<u64>,
    test_before_acquire: Option<bool>,
    ping_after_idle: Option<u64>,
}

impl DatabaseConfigBuilder {
    /// Set the database URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set maximum pool connections
    pub fn max_connections(mut self, count: u32) -> Self {
        self.max_connections = Some(count);
        self
    }

    /// Set minimum pool connections
    pub fn min_connections(mut self, count: u32) -> Self {
        self.min_connections = Some(count);
        self
    }

    /// Set connection timeout in seconds
    pub fn connect_timeout(mut self, seconds: u64) -> Self {
        self.connect_timeout = Some(seconds);
        self
    }

    /// Enable or disable SQL logging
    pub fn logging(mut self, enabled: bool) -> Self {
        self.logging = Some(enabled);
        self
    }

    /// Seconds before an idle pooled connection is closed. `0` disables
    /// idle reaping.
    pub fn idle_timeout(mut self, seconds: u64) -> Self {
        self.idle_timeout = Some(seconds);
        self
    }

    /// Seconds a pooled connection may live before the pool recycles
    /// it. `0` disables lifetime recycling.
    pub fn max_lifetime(mut self, seconds: u64) -> Self {
        self.max_lifetime = Some(seconds);
        self
    }

    /// Seconds to wait for a free pooled connection. Overrides
    /// [`Self::connect_timeout`] for the checkout wait.
    pub fn acquire_timeout(mut self, seconds: u64) -> Self {
        self.acquire_timeout = Some(seconds);
        self
    }

    /// Ping a pooled connection before handing it out.
    pub fn test_before_acquire(mut self, enabled: bool) -> Self {
        self.test_before_acquire = Some(enabled);
        self
    }

    /// Ping a pooled connection only after it has been idle this many
    /// seconds. Setting this disables the per-checkout ping.
    pub fn ping_after_idle(mut self, seconds: u64) -> Self {
        self.ping_after_idle = Some(seconds);
        self
    }

    /// Build the configuration.
    ///
    /// `url`: if [`Self::url`] was called the resulting config
    /// carries [`UrlSource::Explicit`] (production-safe - the
    /// operator chose this URL deliberately). Otherwise the URL +
    /// source are inherited from
    /// [`DatabaseConfig::from_env`] - `Env` when `DATABASE_URL` is
    /// set, `Default` when it falls through to the SQLite
    /// convenience URL.
    pub fn build(self) -> DatabaseConfig {
        let defaults = DatabaseConfig::from_env();
        let (url, url_source) = match self.url {
            Some(u) => (u, UrlSource::Explicit),
            None => (defaults.url, defaults.url_source),
        };
        DatabaseConfig {
            url,
            max_connections: self.max_connections.unwrap_or(defaults.max_connections),
            min_connections: self.min_connections.unwrap_or(defaults.min_connections),
            connect_timeout: self.connect_timeout.unwrap_or(defaults.connect_timeout),
            logging: self.logging.unwrap_or(defaults.logging),
            idle_timeout: self.idle_timeout.or(defaults.idle_timeout),
            max_lifetime: self.max_lifetime.or(defaults.max_lifetime),
            acquire_timeout: self.acquire_timeout.or(defaults.acquire_timeout),
            test_before_acquire: self
                .test_before_acquire
                .unwrap_or(defaults.test_before_acquire),
            ping_after_idle: self.ping_after_idle.or(defaults.ping_after_idle),
            url_source,
        }
    }
}
