//! One open MongoDB connection: the driver's client and the database the
//! connection names.

use std::fmt;
use std::sync::Arc;

use ::mongodb::options::ClientOptions;
use ::mongodb::{Client, Collection, Database};

use super::config::{Labels, MongoConnectionConfig, scrub};
use crate::error::FrameworkError;

/// A MongoDB connection: the driver's [`Client`], with its pool, and the
/// [`Database`] the configuration names.
///
/// Opening it sends nothing to the server: the driver connects on the first
/// operation and keeps reconnecting in the background. So a server that is
/// down does not stop the boot; the first call that needs it, such as
/// [`Self::ping`], fails instead. Clones share one client and one pool.
#[derive(Clone)]
pub struct MongoConnection {
    name: Arc<str>,
    client: Client,
    database: Database,
}

impl MongoConnection {
    /// Open the connection `name` from `config`.
    ///
    /// No server is contacted for a `mongodb://` URI. A `mongodb+srv://`
    /// URI looks up its DNS records here, as the driver requires before it
    /// knows the hosts.
    ///
    /// # Errors
    ///
    /// When a value of `config` is invalid (the error names the
    /// connection), when the SRV lookup fails, or when no Tokio runtime is
    /// running: the driver runs its pool and its server monitors as Tokio
    /// tasks.
    pub async fn connect(
        name: impl Into<String>,
        config: &MongoConnectionConfig,
    ) -> Result<Self, FrameworkError> {
        let name: Arc<str> = Arc::from(name.into());
        let connection_string = config.validate_as(&Labels::of(&name))?;
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(FrameworkError::internal(format!(
                "the MongoDB connection '{name}' needs a Tokio runtime: the driver runs its \
                 pool and its server monitors as Tokio tasks, so open it from inside the \
                 application's runtime"
            )));
        }
        let mut options = ClientOptions::parse(connection_string)
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "the MongoDB connection '{name}' could not use its URI ({})",
                    scrub(&error.kind.to_string(), &config.uri)
                ))
            })?;
        config.apply_pool(&mut options);
        let client = Client::with_options(options).map_err(|error| {
            FrameworkError::internal(format!(
                "the MongoDB connection '{name}' could not start its client ({})",
                scrub(&error.kind.to_string(), &config.uri)
            ))
        })?;
        let database = client.database(&config.database);
        Ok(Self {
            name,
            client,
            database,
        })
    }

    /// The connection's name: [`DEFAULT_MONGO_CONNECTION`](crate::DEFAULT_MONGO_CONNECTION)
    /// for the default one.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The driver's client, for what the database handle does not reach:
    /// other databases, sessions and transactions, and change streams on
    /// the whole deployment.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The database the configuration names. Cheap: it shares the client.
    pub fn database(&self) -> Database {
        self.database.clone()
    }

    /// The collection `name` in the database, reading and writing `T`
    /// through serde. Use [`bson::Document`](crate::bson::Document) for
    /// untyped documents. Nothing is sent to the server until an operation
    /// runs.
    pub fn collection<T: Send + Sync>(&self, name: &str) -> Collection<T> {
        self.database.collection(name)
    }

    /// Send `ping` to the server, as a health check.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached within the server selection
    /// timeout (30 seconds unless the URI or the configuration sets
    /// another), when it refuses the credentials, or when it answers with
    /// an error. The driver's error is the source of the returned error.
    pub async fn ping(&self) -> Result<(), FrameworkError> {
        self.database
            .run_command(::bson::doc! { "ping": 1 })
            .await
            .map(|_| ())
            .map_err(|error| {
                FrameworkError::from_external_with(
                    format!(
                        "the MongoDB connection '{}' could not reach the server: {}",
                        self.name, error.kind
                    ),
                    error,
                )
            })
    }
}

impl fmt::Debug for MongoConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoConnection")
            .field("name", &self.name)
            .field("database", &self.database.name())
            .finish_non_exhaustive()
    }
}
