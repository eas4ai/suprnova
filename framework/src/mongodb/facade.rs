//! The `Mongo` facade: the application's MongoDB connections, registered in
//! the container at boot and reached by name.

use std::collections::BTreeMap;
use std::sync::Arc;

use ::mongodb::{Collection, Database};

use super::config::{DEFAULT_MONGO_CONNECTION, MongoConfig, uri_is_set};
use super::connection::MongoConnection;
use crate::config::Config;
use crate::container::App;
use crate::error::FrameworkError;

/// The open connections, by name, as the container holds them. One value,
/// so a configuration registers whole or not at all.
#[derive(Clone)]
pub(crate) struct MongoConnections {
    by_name: Arc<BTreeMap<String, MongoConnection>>,
}

impl MongoConnections {
    /// Open every connection of `config`. Nothing is sent to a server.
    async fn open(config: &MongoConfig) -> Result<Self, FrameworkError> {
        config.validate()?;
        let mut by_name = BTreeMap::new();
        by_name.insert(
            DEFAULT_MONGO_CONNECTION.to_owned(),
            MongoConnection::connect(DEFAULT_MONGO_CONNECTION, &config.default).await?,
        );
        for (name, connection) in &config.connections {
            by_name.insert(
                name.clone(),
                MongoConnection::connect(name.as_str(), connection).await?,
            );
        }
        Ok(Self {
            by_name: Arc::new(by_name),
        })
    }
}

/// The facade over the application's MongoDB connections, as Laravel's
/// `mongodb` database connection.
///
/// The server and the workers register the connections at boot when
/// `MONGODB_URI` is set or a [`MongoConfig`] is registered with
/// [`Config::register`]; see [`Self::bootstrap`]. Code that runs before
/// the boot, such as the application's own bootstrap hook, calls
/// [`Self::init`] or [`Self::init_with`] first.
///
/// ```rust,no_run
/// use suprnova::Mongo;
/// use suprnova::bson::{Document, doc};
///
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// Mongo::ping().await?;
/// let users = Mongo::collection::<Document>("users")?;
/// let ada = users.find_one(doc! { "name": "Ada" }).await?;
/// # let _ = ada;
/// # Ok(()) }
/// ```
pub struct Mongo;

impl Mongo {
    /// Register the connections of the registered [`MongoConfig`], or of
    /// [`MongoConfig::from_env`] when none is registered, replacing any
    /// registered before.
    ///
    /// # Errors
    ///
    /// As [`MongoConfig::from_env`] and [`MongoConnection::connect`].
    pub async fn init() -> Result<(), FrameworkError> {
        let config = match Config::get::<MongoConfig>() {
            Some(config) => config,
            None => MongoConfig::from_env()?,
        };
        Self::init_with(config).await
    }

    /// Register the connections of `config`, replacing any registered
    /// before.
    ///
    /// # Errors
    ///
    /// As [`MongoConfig::validate`] and [`MongoConnection::connect`].
    pub async fn init_with(config: MongoConfig) -> Result<(), FrameworkError> {
        let connections = MongoConnections::open(&config).await?;
        App::singleton(connections);
        Ok(())
    }

    /// The boot's registration. When no connection is registered yet, it
    /// registers the connections of the registered [`MongoConfig`], or of
    /// the environment when `MONGODB_URI` is set; otherwise it does
    /// nothing. Connections the application registered first are kept.
    ///
    /// The server calls it before it boots the cache and the queue, and so
    /// do the workers and the console, so their MongoDB drivers find the
    /// connection. No server is contacted.
    ///
    /// # Errors
    ///
    /// When `MONGODB_URI` is set but invalid, naming it, and as
    /// [`Self::init_with`].
    pub async fn bootstrap() -> Result<(), FrameworkError> {
        if App::has::<MongoConnections>() {
            return Ok(());
        }
        let config = match Config::get::<MongoConfig>() {
            Some(config) => config,
            None if uri_is_set() => MongoConfig::from_env()?,
            None => return Ok(()),
        };
        let connections = MongoConnections::open(&config).await?;
        App::singleton_if_absent(connections);
        Ok(())
    }

    /// The default connection, [`DEFAULT_MONGO_CONNECTION`].
    ///
    /// # Errors
    ///
    /// When no connection is registered. The error says why: `MONGODB_URI`
    /// unset or invalid (naming it), or set but not yet registered because
    /// the boot has not run.
    pub fn connection() -> Result<MongoConnection, FrameworkError> {
        Self::connection_named(DEFAULT_MONGO_CONNECTION)
    }

    /// The connection `name`: a named connection of the [`MongoConfig`],
    /// or the default one under [`DEFAULT_MONGO_CONNECTION`].
    ///
    /// # Errors
    ///
    /// As [`Self::connection`] when nothing is registered, and when no
    /// registered connection has the name.
    pub fn connection_named(name: &str) -> Result<MongoConnection, FrameworkError> {
        let connections = App::get::<MongoConnections>().ok_or_else(unregistered)?;
        connections.by_name.get(name).cloned().ok_or_else(|| {
            FrameworkError::internal(format!(
                "no MongoDB connection is named '{name}': add it with \
                 MongoConfig::builder().connection(..) and register the configuration"
            ))
        })
    }

    /// The default connection's database.
    ///
    /// # Errors
    ///
    /// As [`Self::connection`].
    pub fn database() -> Result<Database, FrameworkError> {
        Ok(Self::connection()?.database())
    }

    /// The collection `name` in the default connection's database, reading
    /// and writing `T` through serde.
    ///
    /// # Errors
    ///
    /// As [`Self::connection`].
    pub fn collection<T: Send + Sync>(name: &str) -> Result<Collection<T>, FrameworkError> {
        Ok(Self::connection()?.collection(name))
    }

    /// Ping the default connection's server, as a health check.
    ///
    /// # Errors
    ///
    /// As [`Self::connection`], and as [`MongoConnection::ping`] when the
    /// server cannot be reached or answers with an error.
    pub async fn ping() -> Result<(), FrameworkError> {
        Self::connection()?.ping().await
    }
}

/// Why no connection is registered, read from what the boot would read.
fn unregistered() -> FrameworkError {
    if Config::has::<MongoConfig>() {
        return FrameworkError::internal(
            "no MongoDB connection is registered: a MongoConfig is registered, but the boot \
             has not run in this process; call Mongo::init().await before the first query",
        );
    }
    match MongoConfig::from_env() {
        Err(error) => error,
        Ok(_) => FrameworkError::internal(
            "no MongoDB connection is registered: MONGODB_URI is set, but the boot has not \
             run in this process; the server and the workers register the connection when \
             they boot, so code that runs before them calls Mongo::init().await",
        ),
    }
}
