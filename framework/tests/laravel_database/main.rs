//! A Suprnova application on a database Laravel 13 created
//! (`docs/spec/laravel-database.md`, LDB-001 to LDB-012).
//!
//! The tests load, for each engine, the schema Laravel's migrations create
//! on it and the rows Laravel wrote, both committed under `fixtures/` and
//! generated once by `fixtures/generate.sh`. No Laravel application runs at
//! test time: what Laravel's code does with a row Suprnova wrote is
//! reproduced here from Laravel's source, and password hashes are checked
//! with the host `php` (`password_get_info`, `password_verify`).
//!
//! The SQLite tests run with the ordinary suite. The Postgres and MySQL
//! tests are ignored until a throwaway database is named; the mechanism
//! makes one of each and runs them all:
//!
//! ```text
//! scripts/check-laravel-database.sh
//! ```
//!
//! Each test is named for the requirement its falsifier belongs to,
//! `ldb_00N_...`, and ends in the engine it runs on.

mod support;

mod failed_jobs;
mod worker_check;
