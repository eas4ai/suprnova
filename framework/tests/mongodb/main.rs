//! Integration tests for the MongoDB backend (PAR-182 to PAR-188), built
//! only with the `database-mongodb` feature.
//!
//! The tests whose names start with `mongodb_` need a MongoDB server:
//! `MONGODB_TEST_URL` is a `mongodb://` URL that names a throwaway database
//! (with `authSource=admin` when the server needs a user). They are
//! `#[ignore]`d, so a plain run passes without a server; run them with
//! `cargo nextest run -p suprnova --features database-mongodb --test mongodb
//! --run-ignored all`. Every other test runs without a server.
//!
//! The `Mongo` facade keeps its connections in the process container and
//! reads `MONGODB_URI` and `MONGODB_DATABASE`, so a test that boots it runs
//! alone in a child process of this binary, started with exactly the
//! MongoDB environment it needs (see `support::run_alone_with`). No test
//! changes its own environment.

pub mod cache;
pub mod configuration;
pub mod connection;
pub mod documents;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/own_process.rs"]
mod own_process;
pub mod queries;
pub mod queue;
pub mod relations;
pub mod session;
pub mod store_support;
pub mod support;
