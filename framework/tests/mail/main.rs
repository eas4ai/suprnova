//! Integration tests for the `mail` module: one binary per module,
//! one former top-level test file per submodule (folded 2026-09-05).

pub mod address;
pub mod boot;
#[path = "../support/env_lock.rs"]
mod env_lock;
pub mod fake;
pub mod file_transport;
pub mod in_memory;
pub mod log;
pub mod mailable_subject_tera;
pub mod mailgun;
pub mod parity;
pub mod postmark;
pub mod production_fail_closed;
pub mod queue;
pub mod resend;
pub mod sendgrid;
pub mod ses;
pub mod smtp;
pub mod telemetry;
pub mod template;
