//! MongoDB, behind the `database-mongodb` feature: the connections, their
//! configuration, and the [`Mongo`] facade, on the official `mongodb`
//! driver.
//!
//! `MONGODB_URI` and `MONGODB_DATABASE` configure the default connection,
//! as Laravel's `mongodb` connection reads `dsn` and `database`;
//! [`MongoConfig::builder`] configures it in code and adds named
//! connections. The server and the workers register the connections in the
//! container at boot ([`Mongo::bootstrap`]). The driver connects lazily, so
//! a server that is down fails the first call that needs it, not the boot.
//!
//! The driver's own types are re-exported here, so an application names
//! them without a dependency of its own: [`Collection`], [`Database`],
//! [`Client`], and the whole crate as [`driver`]. BSON is
//! [`crate::bson`].

mod config;
mod connection;
mod facade;

pub use config::{
    DEFAULT_MONGO_CONNECTION, MongoConfig, MongoConfigBuilder, MongoConnectionConfig,
};
pub use connection::MongoConnection;
pub use facade::Mongo;

/// The `mongodb` driver crate, for what the names here leave out: options,
/// cursors, sessions, indexes and change streams.
pub use ::mongodb as driver;
/// The driver's client, database and collection handles, which the facade
/// and [`MongoConnection`] answer.
pub use ::mongodb::{Client, Collection, Database};
