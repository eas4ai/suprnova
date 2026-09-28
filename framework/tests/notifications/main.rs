//! Integration tests for the `notifications` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod anonymous;
pub mod broadcast;
pub mod database;
pub mod database_postgres;
pub mod database_read;
pub mod dispatch;
pub mod lifecycle;
pub mod mail;
pub mod mail_derive;
pub mod notify_fake;
pub mod queue;
pub mod telemetry;
pub mod webpush;
