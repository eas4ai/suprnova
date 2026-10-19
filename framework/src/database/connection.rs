//! Database connection management

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::sync::Arc;
use std::time::Duration;

use crate::database::config::DatabaseConfig;
use crate::error::FrameworkError;

/// Wrapper around SeaORM's DatabaseConnection
///
/// This provides a clonable, thread-safe connection that can be stored
/// in the application container and shared across requests.
///
/// # Example
///
/// ```rust,ignore
/// let conn = DbConnection::connect(&config).await?;
///
/// // Use with SeaORM queries
/// let users = User::find().all(conn.inner()).await?;
/// ```
#[derive(Clone)]
pub struct DbConnection {
    inner: Arc<DatabaseConnection>,
    /// Whether every statement of this connection runs inside one open
    /// test transaction, the one [`TestDatabase::refresh`] holds. A
    /// `BEGIN` on it is a savepoint, which cannot change the isolation
    /// level on Postgres or MySQL.
    ///
    /// [`TestDatabase::refresh`]: crate::database::testing::TestDatabase::refresh
    in_test_transaction: bool,
}

impl DbConnection {
    /// Create a new database connection from config
    ///
    /// This establishes a connection pool using the provided configuration.
    /// For SQLite databases, this will automatically create the database file
    /// if it doesn't exist.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, FrameworkError> {
        Self::connect_as(config, crate::database::PRIMARY_CONNECTION_NAME).await
    }

    /// Open a pool and fire `ConnectionEstablished` carrying the
    /// logical name `connection_name`. The public [`Self::connect`]
    /// delegates here with `__primary__`; [`ConnectionRegistry::register`]
    /// calls it directly with the caller-supplied name so multi-pool
    /// observers see the real connection identifier in the event.
    pub(crate) async fn connect_as(
        config: &DatabaseConfig,
        connection_name: &str,
    ) -> Result<Self, FrameworkError> {
        Self::connect_with(config, connection_name, |_| {}).await
    }

    /// Open the pool of a test transaction: `config`'s pool, changed by
    /// `customize`, which installs the hooks that begin the transaction
    /// on the pool's one connection. The connection it returns reports
    /// [`Self::in_test_transaction`].
    pub(crate) async fn connect_test_transaction(
        config: &DatabaseConfig,
        customize: impl FnOnce(&mut ConnectOptions),
    ) -> Result<Self, FrameworkError> {
        let mut connection =
            Self::connect_with(config, crate::database::PRIMARY_CONNECTION_NAME, customize).await?;
        connection.in_test_transaction = true;
        Ok(connection)
    }

    /// Whether every statement of this connection runs inside the open
    /// transaction of a test, so that a `BEGIN` on it is a savepoint.
    pub(crate) fn in_test_transaction(&self) -> bool {
        self.in_test_transaction
    }

    /// [`Self::connect_as`], with `customize` applied to the pool options
    /// after the ones `config` sets.
    async fn connect_with(
        config: &DatabaseConfig,
        connection_name: &str,
        customize: impl FnOnce(&mut ConnectOptions),
    ) -> Result<Self, FrameworkError> {
        // Validate pool config before SeaORM silently accepts a
        // misconfigured `ConnectOptions` (e.g. a zero-sized pool that
        // immediately starves callers).
        config.validate_pool()?;
        config.validate_encoding()?;

        // For SQLite, ensure the database file can be created
        let url = if config.url.starts_with("sqlite://") {
            // Normalize once: separate the file portion from any query
            // string the caller already supplied so filesystem ops never
            // run against a query-polluted path, and rebuild the URL with
            // `mode=rwc` merged exactly once.
            let (path, normalized) = normalize_sqlite_url(&config.url);

            // Don't apply to in-memory databases
            if path != ":memory:" && !path.starts_with(":memory:") {
                // Create parent directories if needed. `?mode=rwc` makes
                // SQLite create the database FILE, but it will not make
                // PARENT DIRECTORIES - propagate any failure here with
                // path context so misconfigured paths and permission
                // problems surface as a clear filesystem diagnostic
                // instead of a downstream "unable to open database file."
                if let Some(parent) = std::path::Path::new(&path).parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        FrameworkError::database(format!(
                            "failed to create SQLite parent directory `{}`: {e}",
                            parent.display(),
                        ))
                    })?;
                }
            }

            normalized
        } else {
            crate::database::config::driver_url(&config.url).into_owned()
        };

        let mut opt = ConnectOptions::new(&url);
        opt.max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .connect_timeout(Duration::from_secs(config.connect_timeout))
            .sqlx_logging(config.logging);

        // Pool liveness. libpq's `keepalives_*` DSN options are not
        // reachable through sqlx 0.9 - its Postgres URL parser knows
        // only sslmode / application_name / options / the statement
        // cache, and neither sqlx-postgres nor sqlx-core carries a TCP
        // keepalive setter. So the answer to "a middlebox killed an idle
        // connection and the pool handed it out anyway" is recycling and
        // pinging, not keeping the socket warm.
        //
        // Each knob is applied only when the operator set it: SeaORM
        // leaving a field `None` means "let sqlx choose", and sqlx's
        // choice (600s idle, 1800s lifetime) is not the same as SeaORM's
        // field default. Setting them unconditionally would silently
        // change every existing deployment's pool.
        if let Some(secs) = config.idle_timeout {
            opt.idle_timeout(nonzero_duration(secs));
        }
        if let Some(secs) = config.max_lifetime {
            opt.max_lifetime(nonzero_duration(secs));
        }
        if let Some(secs) = config.acquire_timeout {
            opt.acquire_timeout(Duration::from_secs(secs));
        }
        // Order matters: `test_before_acquire_if_idle_for` sets
        // `test_before_acquire` to false as a documented side effect, so
        // that the threshold ping is not shadowed by a ping on every
        // acquire. Applying the explicit flag first and the threshold
        // second is what preserves that.
        opt.test_before_acquire(config.test_before_acquire);
        if let Some(secs) = config.ping_after_idle {
            opt.test_before_acquire_if_idle_for(Duration::from_secs(secs));
        }
        // MySQL and MariaDB: every connection of the pool sends
        // `SET NAMES <charset> COLLATE <collation>` when it opens, as
        // Laravel's `MySqlConnector` does, so a string literal compares with
        // the tables' collation instead of the server's default. The config
        // already holds the URL's own parameters when it has them.
        #[cfg(feature = "database-mysql")]
        if let (Some(charset), Some(collation)) = (config.charset.clone(), config.collation.clone())
        {
            opt.map_sqlx_mysql_opts(move |options| options.charset(&charset).collation(&collation));
        }
        // A `customize` that replaces the MySQL pool options replaces this
        // hook too, so it must apply `MysqlSemantics` itself, as the test
        // transaction's does.
        #[cfg(feature = "database-mysql")]
        {
            let semantics = MysqlSemantics::new();
            opt.map_sqlx_mysql_pool_opts(move |pool| {
                let semantics = semantics.clone();
                pool.after_connect(move |connection, _metadata| semantics.apply(connection))
            });
        }
        customize(&mut opt);

        let conn = Database::connect(opt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        let result = Self {
            inner: Arc::new(conn),
            in_test_transaction: false,
        };
        // Fire ConnectionEstablished. The default pool reaches this
        // path with __primary__ semantics; named pools flow through
        // here from ConnectionRegistry::register with their registered
        // name. Listeners observe the connection name; failures are
        // logged-only - a listener bug must not block the pool from
        // coming up.
        let event = crate::database::events::ConnectionEstablished {
            connection_name: connection_name.to_string(),
        };
        if crate::EventFacade::has_listeners::<crate::database::events::ConnectionEstablished>()
            && let Err(e) = crate::EventFacade::dispatch_best_effort(event).await
        {
            tracing::warn!(
                target: "suprnova::database",
                error = %e,
                "ConnectionEstablished listener returned error; ignoring",
            );
        }
        Ok(result)
    }

    /// Wrap an existing SeaORM `DatabaseConnection` as a `DbConnection`.
    ///
    /// Intended for tests and advanced setups that build their own
    /// connection (e.g. in-memory SQLite via `sea_orm::Database::connect`)
    /// and want it visible to `DB::connection()` after registering it
    /// in the container.
    pub fn from_raw(conn: sea_orm::DatabaseConnection) -> Self {
        Self {
            inner: Arc::new(conn),
            in_test_transaction: false,
        }
    }

    /// Get a reference to the underlying SeaORM connection
    ///
    /// Use this when you need to execute raw SeaORM queries.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let conn = DB::connection()?;
    /// let users = User::find()
    ///     .filter(user::Column::Active.eq(true))
    ///     .all(conn.inner())
    ///     .await?;
    /// ```
    pub fn inner(&self) -> &DatabaseConnection {
        &self.inner
    }

    /// Get a reference to the database connection (short alias for `inner()`)
    ///
    /// Use this when passing to SeaORM queries. This provides a cleaner API
    /// than using the `Deref` implementation with `&*`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let users = User::find().all(self.db.conn()).await?;
    /// ```
    pub fn conn(&self) -> &DatabaseConnection {
        &self.inner
    }

    /// Test-only: calls `observer` once for every statement this
    /// connection executes, including statements executed on transactions
    /// started from it. Returns whether the observer was installed.
    ///
    /// `observer` takes nothing and is told nothing: SeaORM's callback
    /// carries the statement text and its bound values, and no test needs
    /// either. A counter is the whole point - a test that can see how many
    /// round trips a request cost can prove that a cache hit costs none.
    ///
    /// Installing needs unique ownership of the pool (SeaORM's
    /// `set_metric_callback` takes `&mut self`), which a connection has
    /// only before it is cloned or shared - in practice immediately after
    /// [`Self::connect`]. `false` means the connection was already shared
    /// and nothing was installed; callers that need the observer must
    /// treat that as a failure rather than as an empty count.
    #[doc(hidden)]
    #[cfg(any(test, feature = "testing"))]
    pub fn observe_statements_for_test<F>(&mut self, observer: F) -> bool
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.observe_statements(observer)
    }

    /// Points SeaORM's metric callback at `observer` and reports whether it
    /// did, which needs sole ownership of the pool.
    ///
    /// `observer` is told nothing: the callback's statement text and bound
    /// values stay here. Shared by
    /// [`StatementCounter`](crate::database::testing::StatementCounter) and
    /// [`Self::observe_statements_for_test`], so the two can never observe
    /// different statements.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn observe_statements<F>(&mut self, observer: F) -> bool
    where
        F: Fn() + Send + Sync + 'static,
    {
        match Arc::get_mut(&mut self.inner) {
            Some(connection) => {
                connection.set_metric_callback(move |_info| observer());
                true
            }
            None => false,
        }
    }
}

/// The statement that gives a MariaDB session MySQL's concurrency
/// semantics. See [`MysqlSemantics`].
#[cfg(feature = "database-mysql")]
const MYSQL_SEMANTICS: &str = "SET SESSION innodb_snapshot_isolation = OFF";

/// `ER_UNKNOWN_SYSTEM_VARIABLE`, MySQL's and MariaDB's answer to a `SET` of
/// a variable the server does not have.
#[cfg(feature = "database-mysql")]
const UNKNOWN_SYSTEM_VARIABLE: u16 = 1193;

/// Gives the new connections of one MySQL-family pool MySQL 8.4's
/// concurrency semantics. Runs once per connection, when the pool opens
/// it, never per query.
///
/// MariaDB 11.8 and later turn `innodb_snapshot_isolation` on by default.
/// Under it, an UPDATE, a DELETE or a locking read in a REPEATABLE READ
/// transaction fails with error 1020, "Record has changed since last read",
/// when another transaction committed a change to the row after this one's
/// first read. MySQL instead waits for the row and works on its latest
/// committed version. The framework's read-then-write sequences, such as
/// spending a single-use token and advancing the RenderCache ledger, are
/// written for MySQL's behavior, so every connection the framework opens
/// turns the setting off for its own session. The server's global value
/// stays as it is for its other clients.
///
/// The server is not asked what it is. MySQL, older MariaDB releases and
/// other MySQL-compatible servers have no such variable and refuse the
/// statement with error 1193, which means there is nothing to turn off.
/// Telling MariaDB apart by its version would cost a round trip of its own
/// on every server, since sqlx keeps only the numbers of the version and
/// not the `MariaDB` in it, and would need a list of the releases that have
/// the variable.
///
/// The setting never fails a connection. A MySQL-compatible proxy or
/// managed service may refuse an unknown variable with another error, and
/// an application that cannot connect at all is far worse off than one
/// whose MariaDB keeps the server's own concurrency behavior. Such a
/// refusal is logged at warn once per pool, and the connection is handed
/// out.
///
/// Clones share one pool's state, so make one per pool.
#[cfg(feature = "database-mysql")]
#[derive(Clone)]
pub(crate) struct MysqlSemantics {
    statement: &'static str,
    /// Whether this pool has warned that a connection kept the server's
    /// setting, so the warning comes once and not with every connection.
    warned: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "database-mysql")]
impl MysqlSemantics {
    /// The semantics of a new pool.
    pub(crate) fn new() -> Self {
        Self::running(MYSQL_SEMANTICS)
    }

    /// A pool whose connections run `statement`: the real one, or in a
    /// test one the server refuses.
    fn running(statement: &'static str) -> Self {
        Self {
            statement,
            warned: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// The pool's connect hook: runs the statement on `connection`.
    pub(crate) fn apply<'c>(
        &self,
        connection: &'c mut sea_orm::sqlx::mysql::MySqlConnection,
    ) -> futures::future::BoxFuture<'c, Result<(), sea_orm::sqlx::Error>> {
        let semantics = self.clone();
        Box::pin(async move {
            let outcome = sea_orm::sqlx::Executor::execute(connection, semantics.statement).await;
            semantics.settle(outcome.map(|_| ()))
        })
    }

    /// The hook's answer to the statement's outcome, which is always to
    /// hand the connection out. A 1193 means there was nothing to turn off
    /// and says nothing. Any other refusal warns, once per pool. The error
    /// is the server's or the driver's answer to the statement and carries
    /// neither the URL nor a credential.
    fn settle(
        &self,
        outcome: Result<(), sea_orm::sqlx::Error>,
    ) -> Result<(), sea_orm::sqlx::Error> {
        match outcome {
            Ok(()) => {}
            Err(error) if refused_as_unknown_variable(&error) => {}
            Err(error) => {
                if !self.warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(
                        target: "suprnova::database",
                        error = %error,
                        "could not turn innodb_snapshot_isolation off for this pool's \
                         connections; on MariaDB 11.8 and later, concurrent transactions \
                         keep the server's own behavior, where an update of a row another \
                         transaction changed after this one's first read fails with error 1020",
                    );
                } else {
                    tracing::debug!(
                        target: "suprnova::database",
                        error = %error,
                        "could not turn innodb_snapshot_isolation off for this connection",
                    );
                }
            }
        }
        Ok(())
    }
}

/// Whether the server refused a statement because it has no such system
/// variable.
#[cfg(feature = "database-mysql")]
fn refused_as_unknown_variable(error: &sea_orm::sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.try_downcast_ref::<sea_orm::sqlx::mysql::MySqlDatabaseError>())
        .is_some_and(|error| error.number() == UNKNOWN_SYSTEM_VARIABLE)
}

/// Map a seconds value onto the `Option<Duration>` sqlx wants for the
/// two connection-reaping knobs.
///
/// sqlx spells "never reap" as `None`, and the environment reader that
/// produces these values cannot express a nested option, so `0` is the
/// disable signal - documented on `DB_IDLE_TIMEOUT` and
/// `DB_MAX_LIFETIME`.
fn nonzero_duration(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

/// Normalize a `sqlite://` URL into `(file_path, connect_url)`.
///
/// Splits the file portion from any caller-supplied query string so
/// filesystem ops only ever see the file, strips a leading `./`, and
/// rebuilds the connect URL with `mode=rwc` merged exactly once. An
/// input that already carries a query (`sqlite://file.db?cache=shared`)
/// keeps that query intact instead of being double-suffixed.
///
/// In-memory targets (`:memory:`) are returned with their query
/// preserved verbatim - `mode=rwc` is meaningless there and SQLite
/// rejects an unexpected `?mode=rwc` on `:memory:`.
pub(crate) fn normalize_sqlite_url(url: &str) -> (String, String) {
    let without_scheme = url.trim_start_matches("sqlite://");
    let (raw_path, query) = match without_scheme.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (without_scheme, None),
    };
    let path = raw_path.trim_start_matches("./");

    if path == ":memory:" || path.starts_with(":memory:") {
        let connect_url = match query {
            Some(q) => format!("sqlite:{path}?{q}"),
            None => format!("sqlite:{path}"),
        };
        return (path.to_string(), connect_url);
    }

    (
        path.to_string(),
        format!("sqlite:{}?{}", path, merge_rwc_query(query)),
    )
}

/// Merge `mode=rwc` into an optional SQLite query string exactly once.
///
/// SQLite needs `mode=rwc` to create the database file on connect. We
/// must add it without (a) duplicating it when the caller already
/// supplied it and (b) clobbering a deliberate `mode=` the caller set
/// (e.g. `mode=ro`). Any other params (`cache=shared`, …) are preserved
/// verbatim and in order.
fn merge_rwc_query(query: Option<&str>) -> String {
    let existing = query.unwrap_or("");
    if existing.is_empty() {
        return "mode=rwc".to_string();
    }
    // If a `mode=` param is already present, the caller's choice wins -
    // appending a second `mode=` would be ambiguous to SQLite.
    let has_mode = existing
        .split('&')
        .any(|pair| pair == "mode" || pair.starts_with("mode="));
    if has_mode {
        existing.to_string()
    } else {
        format!("{existing}&mode=rwc")
    }
}

// Domain 6 audit D6-3 - `pub fn is_closed(&self) -> bool` was removed.
// The previous implementation hardcoded `false` with a comment saying
// "SeaORM doesn't expose this directly, but we can check via ping"; in
// practice it lied about every connection's state. `ping().is_err()`
// would also conflate transient network blips with closed-state and
// would still leave the public name `is_closed` semantically wrong (a
// failed ping is "unhealthy", not "closed"). With one caller - a
// tautological `assert!(!conn.is_closed())` in the registry round-trip
// test - removing the method is cleaner than papering over it.

impl AsRef<DatabaseConnection> for DbConnection {
    fn as_ref(&self) -> &DatabaseConnection {
        &self.inner
    }
}

impl std::ops::Deref for DbConnection {
    type Target = DatabaseConnection;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(all(test, feature = "database-mysql"))]
mod mysql_semantics_tests {
    use super::MysqlSemantics;

    /// How a MySQL-compatible proxy or service might refuse the statement:
    /// with anything but MySQL's own error 1193.
    fn refused(reason: &str) -> Result<(), sea_orm::sqlx::Error> {
        Err(sea_orm::sqlx::Error::Protocol(reason.to_owned()))
    }

    fn warnings(lines: &[&str]) -> usize {
        lines
            .iter()
            .filter(|line| line.contains("WARN") && line.contains("innodb_snapshot_isolation"))
            .count()
    }

    /// The setting is worth having, never worth failing a connection over:
    /// a refusal other than 1193 still hands the connection out, and the
    /// pool says so once, however many of its connections are refused.
    #[test]
    #[tracing_test::traced_test]
    fn a_refused_setting_still_opens_the_connection_and_warns_once_per_pool() {
        let pool = MysqlSemantics::new();
        pool.settle(refused("unsupported system variable"))
            .expect("the first refused connection is handed out");
        pool.clone()
            .settle(refused("unsupported system variable"))
            .expect("a second refused connection of the pool is handed out");
        logs_assert(|lines| match warnings(lines) {
            1 => Ok(()),
            seen => Err(format!("one pool warns once, saw {seen} warnings")),
        });

        MysqlSemantics::new()
            .settle(refused("unsupported system variable"))
            .expect("another pool's refused connection is handed out");
        logs_assert(|lines| match warnings(lines) {
            2 => Ok(()),
            seen => Err(format!(
                "a second pool warns once more, saw {seen} warnings"
            )),
        });
    }

    /// A real refusal on MariaDB: a value the variable does not take fails
    /// with error 1231, and the connection still opens and runs queries.
    #[tokio::test]
    #[ignore = "requires disposable MariaDB at MARIADB_TEST_URL"]
    async fn mariadb_a_refused_setting_still_gives_a_working_connection() {
        let url = std::env::var("MARIADB_TEST_URL").expect("set MARIADB_TEST_URL");
        let semantics =
            MysqlSemantics::running("SET SESSION innodb_snapshot_isolation = 'neither'");
        let pool = sea_orm::sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(2)
            .after_connect(move |connection, _metadata| semantics.apply(connection))
            .connect(&crate::database::config::driver_url(&url))
            .await
            .expect("a refused setting does not fail the connection");
        let first = pool.acquire().await.expect("a first connection");
        let mut second = pool.acquire().await.expect("a second connection");
        let (session, global): (i64, i64) = sea_orm::sqlx::query_as(
            "SELECT @@SESSION.innodb_snapshot_isolation, @@GLOBAL.innodb_snapshot_isolation",
        )
        .fetch_one(&mut *second)
        .await
        .expect("the connection runs queries");
        assert_eq!(session, global, "the refused statement changed nothing");
        drop((first, second));
        pool.close().await;
    }
}

#[cfg(test)]
mod sqlite_url_tests {
    use super::normalize_sqlite_url;

    #[test]
    fn plain_path_gets_single_rwc() {
        let (path, url) = normalize_sqlite_url("sqlite://database.sqlite");
        assert_eq!(path, "database.sqlite");
        assert_eq!(url, "sqlite:database.sqlite?mode=rwc");
    }

    #[test]
    fn leading_dot_slash_is_stripped() {
        let (path, url) = normalize_sqlite_url("sqlite://./database.db");
        assert_eq!(path, "database.db");
        assert_eq!(url, "sqlite:database.db?mode=rwc");
    }

    #[test]
    fn existing_mode_rwc_is_not_duplicated() {
        // The bug: `sqlite://file.db?mode=rwc` previously produced
        // `sqlite:file.db?mode=rwc?mode=rwc` and a query-polluted path.
        let (path, url) = normalize_sqlite_url("sqlite://file.db?mode=rwc");
        assert_eq!(path, "file.db", "filesystem path must be query-free");
        assert_eq!(url, "sqlite:file.db?mode=rwc");
        // Exactly one query separator and one mode param.
        assert_eq!(url.matches('?').count(), 1);
        assert_eq!(url.matches("mode=rwc").count(), 1);
    }

    #[test]
    fn other_query_params_are_preserved_with_rwc_merged() {
        let (path, url) = normalize_sqlite_url("sqlite://file.db?cache=shared");
        assert_eq!(path, "file.db", "filesystem path must be query-free");
        assert_eq!(url, "sqlite:file.db?cache=shared&mode=rwc");
        assert_eq!(url.matches('?').count(), 1);
        assert_eq!(url.matches("mode=rwc").count(), 1);
    }

    #[test]
    fn caller_mode_choice_is_not_clobbered() {
        // A deliberate `mode=ro` must win - we don't append a conflicting
        // second `mode=`.
        let (path, url) = normalize_sqlite_url("sqlite://file.db?mode=ro");
        assert_eq!(path, "file.db");
        assert_eq!(url, "sqlite:file.db?mode=ro");
        assert_eq!(url.matches("mode=").count(), 1);
    }

    #[test]
    fn in_memory_is_left_alone() {
        let (path, url) = normalize_sqlite_url("sqlite://:memory:");
        assert_eq!(path, ":memory:");
        assert_eq!(url, "sqlite::memory:");
    }

    #[test]
    fn in_memory_with_query_preserves_query_without_rwc() {
        let (path, url) = normalize_sqlite_url("sqlite://:memory:?cache=shared");
        assert_eq!(path, ":memory:");
        assert_eq!(url, "sqlite::memory:?cache=shared");
        assert!(!url.contains("mode=rwc"));
    }
}
