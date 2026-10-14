//! Search core behavior and driver conformance share one integration binary.
#[cfg(feature = "testing")]
mod builder;
#[cfg(feature = "testing")]
mod commands;
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
#[cfg(feature = "testing")]
mod generations;
mod highlight;
#[cfg(feature = "testing")]
mod indexing;
mod query;

#[cfg(feature = "search-fastembed")]
mod embedder_fastembed;
mod embedder_http;
mod embedder_http_openrouter;
#[cfg(feature = "testing")]
mod qdrant;
