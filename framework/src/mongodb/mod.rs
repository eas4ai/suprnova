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
//!
//! Document models (PAR-183) are structs `#[suprnova::document]` stores in
//! a collection, with the Eloquent calls of [`DocumentModel`];
//! [`DocumentQuery`] (PAR-184) queries them, and [`events`] holds their
//! lifecycle events.

mod config;
mod connection;
mod document;
pub mod events;
mod facade;
// The helpers the MongoDB queue, cache and session stores share.
pub(crate) mod stores;
mod query;

pub use config::{
    DEFAULT_MONGO_CONNECTION, MongoConfig, MongoConfigBuilder, MongoConnectionConfig,
};
pub use connection::MongoConnection;
pub use document::{
    AsBsonDateTime, AsDecimal128, DocumentCast, DocumentKey, DocumentModel, EmbedsMany, EmbedsOne,
};
// What the code `#[suprnova::document]` emits calls; not part of the API.
#[doc(hidden)]
pub use document::{
    __read_cast_field, __read_field, __resolve_document_route_binding, __serialize_document,
    __write_cast_field, __write_field,
};
pub use events::DocumentObserver;
pub use facade::Mongo;
pub use query::{DocumentGroup, DocumentQuery, RenderedFind};

/// The `mongodb` driver crate, for what the names here leave out: options,
/// cursors, sessions, indexes and change streams.
pub use ::mongodb as driver;
/// The driver's client, database and collection handles, which the facade
/// and [`MongoConnection`] answer.
pub use ::mongodb::{Client, Collection, Database};
