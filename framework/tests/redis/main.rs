//! Integration tests for the `Redis` facade (PAR-031 to PAR-034).
//!
//! They need a Redis server: `REDIS_TEST_URL` names a database the tests
//! may write to, such as `redis://127.0.0.1:6379/9`. Every test is
//! `#[ignore]`d so a plain `cargo test` passes without one; run them with
//! `cargo test -p suprnova --test redis -- --ignored --test-threads=1`.
//! The connections and the event listeners are process-global, so every
//! test is `#[serial]`, and each uses keys and connection names of its own.

pub mod commands;
pub mod connections;
pub mod pipelines;
pub mod subscriptions;
pub mod support;
