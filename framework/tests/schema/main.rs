//! Integration tests for `suprnova::schema`, the opt-in migration builder.
//!
//! The cases live once in `cases.rs`, as functions over a
//! `DatabaseConnection`, and each backend file calls them, so SQLite,
//! Postgres and MySQL run the same checks. The SQLite tests run with the
//! ordinary suite. The Postgres and MySQL tests are ignored until a
//! disposable database is named:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test schema postgres_ -- --ignored --test-threads=1
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test schema mysql_ -- --ignored --test-threads=1
//! ```
//!
//! Explicit execution without the URL fails immediately; it never reports a
//! silent pass.

mod cases;
mod catalog;
mod laravel_cases;
mod mysql;
mod postgres;
mod sqlite;
