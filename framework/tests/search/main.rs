//! Search core behavior and driver conformance share one integration binary.
#[cfg(feature = "testing")]
mod builder;
mod core;
#[cfg(feature = "testing")]
mod database;
#[cfg(feature = "testing")]
mod database_mysql;
#[cfg(feature = "testing")]
mod database_postgres;
#[cfg(feature = "testing")]
mod database_sqlite;
#[cfg(feature = "testing")]
mod database_support;
#[cfg(feature = "testing")]
mod derive;
mod embedder;
mod engine;
mod expansion;
mod fusion;
mod highlight;
#[cfg(feature = "testing")]
mod indexing;
mod query;
