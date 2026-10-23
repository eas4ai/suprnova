//! The MongoDB connection settings: `MONGODB_URI` and `MONGODB_DATABASE`,
//! as Laravel's `mongodb` connection reads `dsn` and `database`, or a
//! builder for configuration in code and for named connections.
//!
//! Everything here is checked without contacting a server. A URI may hold a
//! password, so no error and no `Debug` output shows it.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use ::mongodb::options::{ClientOptions, ConnectionString};

use crate::error::FrameworkError;

/// The name of the connection [`Mongo::connection`](crate::Mongo::connection)
/// answers, as Laravel's `config/database.php` names its MongoDB connection.
/// [`Mongo::connection_named`](crate::Mongo::connection_named) answers it by
/// this name too.
pub const DEFAULT_MONGO_CONNECTION: &str = "mongodb";

/// The variable that holds the default connection's URI.
const URI_VARIABLE: &str = "MONGODB_URI";

/// The variable that holds the default connection's database.
const DATABASE_VARIABLE: &str = "MONGODB_DATABASE";

/// The longest database name MongoDB accepts, in bytes.
const MAX_DATABASE_NAME_BYTES: usize = 63;

/// The characters MongoDB refuses in a database name on every platform.
const ILLEGAL_DATABASE_CHARACTERS: [char; 7] = ['/', '\\', '.', ' ', '"', '$', '\0'];

/// One MongoDB connection: the server's URI, the database the application
/// uses there, and the pool options of the driver.
///
/// A pool option left `None` keeps what the URI's query string says, and
/// the driver's default when the URI says nothing; a value here wins over
/// the URI. The values are checked when a [`MongoConfig`] is built and when
/// the connection opens, not when this struct is made.
///
/// `Debug` hides the URI's user, password and query string.
#[derive(Clone)]
pub struct MongoConnectionConfig {
    /// The connection string, `mongodb://` or `mongodb+srv://`.
    pub uri: String,
    /// The database [`MongoConnection::database`](crate::MongoConnection::database)
    /// answers.
    pub database: String,
    /// The most connections the pool holds to one server (the driver's
    /// `maxPoolSize`, 10 by default). At least 1.
    pub max_pool_size: Option<u32>,
    /// The connections the pool keeps open to one server even when idle
    /// (`minPoolSize`, 0 by default). At most `max_pool_size`.
    pub min_pool_size: Option<u32>,
    /// How long a connection may sit idle in the pool before it is closed
    /// (`maxIdleTimeMS`).
    pub max_idle_time: Option<Duration>,
    /// How many connections the pool opens at once (`maxConnecting`, 2 by
    /// default). At least 1.
    pub max_connecting: Option<u32>,
    /// How long opening one connection may take (`connectTimeoutMS`).
    pub connect_timeout: Option<Duration>,
    /// How long an operation waits for a server it can use before it fails
    /// (`serverSelectionTimeoutMS`, 30 seconds by default). This bounds how
    /// long the first call to an unreachable server takes to fail.
    pub server_selection_timeout: Option<Duration>,
}

impl MongoConnectionConfig {
    /// A connection to `database` on the server `uri` names, with the pool
    /// options the URI gives.
    pub fn new(uri: impl Into<String>, database: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            database: database.into(),
            max_pool_size: None,
            min_pool_size: None,
            max_idle_time: None,
            max_connecting: None,
            connect_timeout: None,
            server_selection_timeout: None,
        }
    }

    /// Check every value as the connection `labels` names, and answer the
    /// parsed URI so the caller does not parse it again.
    pub(crate) fn validate_as(&self, labels: &Labels) -> Result<ConnectionString, FrameworkError> {
        let connection_string = parse_uri(&self.uri, &labels.uri)?;
        validate_database_name(&self.database, &labels.database)?;
        self.validate_pool(&labels.connection)?;
        Ok(connection_string)
    }

    /// Refuse the pool options the driver would refuse when it opens the
    /// connection, with the option named.
    fn validate_pool(&self, connection: &str) -> Result<(), FrameworkError> {
        if self.max_pool_size == Some(0) {
            return Err(FrameworkError::internal(format!(
                "{connection}: max_pool_size must be at least 1; a pool of no connections \
                 never answers"
            )));
        }
        if self.max_connecting == Some(0) {
            return Err(FrameworkError::internal(format!(
                "{connection}: max_connecting must be at least 1; a pool that opens no \
                 connections never answers"
            )));
        }
        if let (Some(min), Some(max)) = (self.min_pool_size, self.max_pool_size)
            && min > max
        {
            return Err(FrameworkError::internal(format!(
                "{connection}: min_pool_size ({min}) is above max_pool_size ({max})"
            )));
        }
        Ok(())
    }

    /// Put the pool options that are set over the ones the URI gave.
    pub(crate) fn apply_pool(&self, options: &mut ClientOptions) {
        if self.max_pool_size.is_some() {
            options.max_pool_size = self.max_pool_size;
        }
        if self.min_pool_size.is_some() {
            options.min_pool_size = self.min_pool_size;
        }
        if self.max_idle_time.is_some() {
            options.max_idle_time = self.max_idle_time;
        }
        if self.max_connecting.is_some() {
            options.max_connecting = self.max_connecting;
        }
        if self.connect_timeout.is_some() {
            options.connect_timeout = self.connect_timeout;
        }
        if self.server_selection_timeout.is_some() {
            options.server_selection_timeout = self.server_selection_timeout;
        }
    }
}

impl fmt::Debug for MongoConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoConnectionConfig")
            .field("uri", &redacted_uri(&self.uri))
            .field("database", &self.database)
            .field("max_pool_size", &self.max_pool_size)
            .field("min_pool_size", &self.min_pool_size)
            .field("max_idle_time", &self.max_idle_time)
            .field("max_connecting", &self.max_connecting)
            .field("connect_timeout", &self.connect_timeout)
            .field("server_selection_timeout", &self.server_selection_timeout)
            .finish()
    }
}

/// The MongoDB connections of an application: the default one and any
/// named ones.
///
/// [`Self::from_env`] reads the default connection from `MONGODB_URI` and
/// `MONGODB_DATABASE`. [`Self::builder`] sets it in code, adds pool options
/// and named connections, and falls back to the same variables for what it
/// does not set. Both check every value, so a built configuration opens
/// without a configuration error; the server is first contacted by the
/// first call that needs it.
///
/// ```rust,no_run
/// use std::time::Duration;
/// use suprnova::{Config, MongoConfig, MongoConnectionConfig};
///
/// # fn ex() -> Result<(), suprnova::FrameworkError> {
/// let config = MongoConfig::builder()
///     .uri("mongodb://127.0.0.1:27017")
///     .database("shop")
///     .max_pool_size(20)
///     .server_selection_timeout(Duration::from_secs(5))
///     .connection(
///         "analytics",
///         MongoConnectionConfig::new("mongodb://analytics.internal:27017", "events"),
///     )
///     .build()?;
/// Config::register(config);
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct MongoConfig {
    /// The connection named [`DEFAULT_MONGO_CONNECTION`], the one
    /// [`Mongo::connection`](crate::Mongo::connection) answers.
    pub default: MongoConnectionConfig,
    /// The other connections, by name.
    pub connections: BTreeMap<String, MongoConnectionConfig>,
}

impl MongoConfig {
    /// The default connection from `MONGODB_URI` and `MONGODB_DATABASE`.
    ///
    /// The database is `MONGODB_DATABASE`, else the database in the URI's
    /// path (`mongodb://host/shop`), as `laravel-mongodb` reads it. Pool
    /// options come from the URI's query string, such as `?maxPoolSize=20`.
    ///
    /// # Errors
    ///
    /// When `MONGODB_URI` is unset, blank or not a MongoDB connection
    /// string, the error names `MONGODB_URI`; when no database is named,
    /// or the name is not one MongoDB accepts, it names
    /// `MONGODB_DATABASE`. The URI's password never appears in it.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::builder().build()
    }

    /// A builder for the configuration in code.
    pub fn builder() -> MongoConfigBuilder {
        MongoConfigBuilder::default()
    }

    /// The connection named `name`: the default one under
    /// [`DEFAULT_MONGO_CONNECTION`], a named one otherwise.
    pub fn connection(&self, name: &str) -> Option<&MongoConnectionConfig> {
        if name == DEFAULT_MONGO_CONNECTION {
            Some(&self.default)
        } else {
            self.connections.get(name)
        }
    }

    /// Check every connection, for a configuration made by hand rather than
    /// built. [`Mongo::init_with`](crate::Mongo::init_with) calls it.
    ///
    /// # Errors
    ///
    /// When a URI is not a MongoDB connection string, a database name is
    /// not one MongoDB accepts, a pool option is one the driver refuses, or
    /// a named connection is called [`DEFAULT_MONGO_CONNECTION`] or
    /// nothing. The error names the connection.
    pub fn validate(&self) -> Result<(), FrameworkError> {
        self.default
            .validate_as(&Labels::of(DEFAULT_MONGO_CONNECTION))?;
        for (name, connection) in &self.connections {
            validate_connection_name(name)?;
            connection.validate_as(&Labels::of(name))?;
        }
        Ok(())
    }
}

/// Builds a [`MongoConfig`]. The default connection's URI and database fall
/// back to `MONGODB_URI` and `MONGODB_DATABASE` when they are not set here,
/// so `MongoConfig::builder().max_pool_size(50).build()` is the environment
/// with a larger pool.
#[derive(Default)]
pub struct MongoConfigBuilder {
    uri: Option<String>,
    database: Option<String>,
    max_pool_size: Option<u32>,
    min_pool_size: Option<u32>,
    max_idle_time: Option<Duration>,
    max_connecting: Option<u32>,
    connect_timeout: Option<Duration>,
    server_selection_timeout: Option<Duration>,
    connections: BTreeMap<String, MongoConnectionConfig>,
}

impl MongoConfigBuilder {
    /// The default connection's URI, over `MONGODB_URI`.
    pub fn uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    /// The default connection's database, over `MONGODB_DATABASE` and the
    /// database in the URI.
    pub fn database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// The most connections the default connection's pool holds to one
    /// server. See [`MongoConnectionConfig::max_pool_size`].
    pub fn max_pool_size(mut self, size: u32) -> Self {
        self.max_pool_size = Some(size);
        self
    }

    /// The connections the default connection's pool keeps open when
    /// idle. See [`MongoConnectionConfig::min_pool_size`].
    pub fn min_pool_size(mut self, size: u32) -> Self {
        self.min_pool_size = Some(size);
        self
    }

    /// How long a pooled connection may sit idle. See
    /// [`MongoConnectionConfig::max_idle_time`].
    pub fn max_idle_time(mut self, time: Duration) -> Self {
        self.max_idle_time = Some(time);
        self
    }

    /// How many connections the pool opens at once. See
    /// [`MongoConnectionConfig::max_connecting`].
    pub fn max_connecting(mut self, count: u32) -> Self {
        self.max_connecting = Some(count);
        self
    }

    /// How long opening one connection may take. See
    /// [`MongoConnectionConfig::connect_timeout`].
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// How long an operation waits for a usable server. See
    /// [`MongoConnectionConfig::server_selection_timeout`].
    pub fn server_selection_timeout(mut self, timeout: Duration) -> Self {
        self.server_selection_timeout = Some(timeout);
        self
    }

    /// Add the connection `name`, which
    /// [`Mongo::connection_named`](crate::Mongo::connection_named) answers.
    /// A second connection of the same name replaces the first.
    pub fn connection(mut self, name: impl Into<String>, config: MongoConnectionConfig) -> Self {
        self.connections.insert(name.into(), config);
        self
    }

    /// Check every value and build the configuration.
    ///
    /// # Errors
    ///
    /// As [`MongoConfig::from_env`] for the default connection's URI and
    /// database, naming the variable when the value came from one, and as
    /// [`MongoConfig::validate`] for the rest.
    pub fn build(self) -> Result<MongoConfig, FrameworkError> {
        let mut labels = Labels::of(DEFAULT_MONGO_CONNECTION);
        let uri = match self.uri {
            Some(uri) => uri,
            None => {
                labels.uri = URI_VARIABLE.to_owned();
                read_variable(URI_VARIABLE)?.ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "{URI_VARIABLE} is not set: set it to the MongoDB connection string, \
                         such as mongodb://127.0.0.1:27017, or give the URI to \
                         MongoConfig::builder().uri(..)"
                    ))
                })?
            }
        };
        let database = match self.database {
            Some(database) => database,
            None => match read_variable(DATABASE_VARIABLE)? {
                Some(database) => {
                    labels.database = DATABASE_VARIABLE.to_owned();
                    database
                }
                None => {
                    labels.database = format!("the database in {}", labels.uri);
                    parse_uri(&uri, &labels.uri)?
                        .default_database
                        .ok_or_else(|| {
                            FrameworkError::internal(format!(
                                "{DATABASE_VARIABLE} is not set and {} names no database: \
                                 set {DATABASE_VARIABLE} to the database the application uses",
                                labels.uri
                            ))
                        })?
                }
            },
        };
        let default = MongoConnectionConfig {
            uri,
            database,
            max_pool_size: self.max_pool_size,
            min_pool_size: self.min_pool_size,
            max_idle_time: self.max_idle_time,
            max_connecting: self.max_connecting,
            connect_timeout: self.connect_timeout,
            server_selection_timeout: self.server_selection_timeout,
        };
        default.validate_as(&labels)?;
        for (name, connection) in &self.connections {
            validate_connection_name(name)?;
            connection.validate_as(&Labels::of(name))?;
        }
        Ok(MongoConfig {
            default,
            connections: self.connections,
        })
    }
}

impl fmt::Debug for MongoConfigBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoConfigBuilder")
            .field("uri", &self.uri.as_deref().map(redacted_uri))
            .field("database", &self.database)
            .field("max_pool_size", &self.max_pool_size)
            .field("min_pool_size", &self.min_pool_size)
            .field("max_idle_time", &self.max_idle_time)
            .field("max_connecting", &self.max_connecting)
            .field("connect_timeout", &self.connect_timeout)
            .field("server_selection_timeout", &self.server_selection_timeout)
            .field("connections", &self.connections)
            .finish()
    }
}

/// How an error names a connection's settings: by the variable a value
/// came from, or by the connection it belongs to.
pub(crate) struct Labels {
    pub(crate) uri: String,
    pub(crate) database: String,
    pub(crate) connection: String,
}

impl Labels {
    /// The labels of the connection `name`, for values set in code.
    pub(crate) fn of(name: &str) -> Self {
        Self {
            uri: format!("the URI of the MongoDB connection '{name}'"),
            database: format!("the database of the MongoDB connection '{name}'"),
            connection: format!("the MongoDB connection '{name}'"),
        }
    }
}

/// Whether `MONGODB_URI` holds anything, so the boot knows there is a
/// connection to register. A value that is not UTF-8 counts, so that the
/// boot reports it rather than skipping it.
pub(crate) fn uri_is_set() -> bool {
    std::env::var_os(URI_VARIABLE).is_some_and(|value| !value.to_string_lossy().trim().is_empty())
}

/// The variable `name`, trimmed, or `None` when it is unset or blank.
fn read_variable(name: &str) -> Result<Option<String>, FrameworkError> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(FrameworkError::internal(format!(
            "{name} is not valid UTF-8"
        ))),
    }
}

/// `uri` parsed as a MongoDB connection string, without contacting a
/// server. The error names `label` and gives the driver's reason with
/// every secret of the URI taken out.
fn parse_uri(uri: &str, label: &str) -> Result<ConnectionString, FrameworkError> {
    ConnectionString::parse(uri).map_err(|error| {
        FrameworkError::internal(format!(
            "{label} is not a MongoDB connection string ({}); it starts with mongodb:// or \
             mongodb+srv:// and names the server's host",
            scrub(&error.kind.to_string(), uri)
        ))
    })
}

/// Refuse a name MongoDB would refuse for a database, before any server
/// sees it.
fn validate_database_name(database: &str, label: &str) -> Result<(), FrameworkError> {
    let reason = if database.is_empty() {
        Some("it is empty".to_owned())
    } else if database.len() > MAX_DATABASE_NAME_BYTES {
        Some(format!(
            "it is {} bytes long, and MongoDB takes at most {MAX_DATABASE_NAME_BYTES}",
            database.len()
        ))
    } else {
        database
            .chars()
            .find(|c| ILLEGAL_DATABASE_CHARACTERS.contains(c))
            .map(|c| format!("it holds {c:?}, which MongoDB refuses in a database name"))
    };
    match reason {
        Some(reason) => Err(FrameworkError::internal(format!(
            "{label} is not a MongoDB database name: {reason}"
        ))),
        None => Ok(()),
    }
}

/// Refuse a connection name that cannot be told from the default one.
fn validate_connection_name(name: &str) -> Result<(), FrameworkError> {
    if name.is_empty() {
        return Err(FrameworkError::internal(
            "a MongoDB connection needs a name: MongoConfigBuilder::connection was given an \
             empty one",
        ));
    }
    if name == DEFAULT_MONGO_CONNECTION {
        return Err(FrameworkError::internal(format!(
            "'{DEFAULT_MONGO_CONNECTION}' is the default MongoDB connection's name: set that \
             connection with MongoConfigBuilder::uri and ::database, and give another name to \
             MongoConfigBuilder::connection"
        )));
    }
    Ok(())
}

/// `reason`, a message of the driver about `uri`, with the URI's user
/// information and query values taken out: the driver quotes parts of the
/// URI in some messages, and those parts can be a password or a token.
pub(crate) fn scrub(reason: &str, uri: &str) -> String {
    let rest = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let (before_query, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut secrets: Vec<String> = Vec::new();
    if let Some((user_info, _)) = before_query.rsplit_once('@') {
        secrets.push(user_info.to_owned());
        if let Some((_, password)) = user_info.split_once(':') {
            secrets.push(password.to_owned());
        }
    }
    for pair in query.split('&') {
        secrets.push(
            pair.split_once('=')
                .map_or(pair, |(_, value)| value)
                .to_owned(),
        );
    }
    let decoded: Vec<String> = secrets
        .iter()
        .map(|secret| {
            percent_encoding::percent_decode_str(secret)
                .decode_utf8_lossy()
                .into_owned()
        })
        .collect();
    secrets.extend(decoded);
    // Short values (`true`, `20`) are settings, not secrets, and replacing
    // them would garble the message. The longest go first, so a secret is
    // replaced whole rather than around a shorter one inside it.
    secrets.retain(|secret| secret.len() >= 3);
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut scrubbed = reason.to_owned();
    for secret in &secrets {
        scrubbed = scrubbed.replace(secret.as_str(), "***");
    }
    scrubbed
}

/// `uri` for a `Debug` line: the scheme, hosts and path, with the user
/// information and the query string replaced by `***`.
fn redacted_uri(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return "***".to_owned();
    };
    let (before_query, query) = match rest.split_once('?') {
        Some((before_query, _)) => (before_query, "?***"),
        None => (rest, ""),
    };
    match before_query.rsplit_once('@') {
        Some((_, hosts)) => format!("{scheme}://***@{hosts}{query}"),
        None => format!("{scheme}://{before_query}{query}"),
    }
}
