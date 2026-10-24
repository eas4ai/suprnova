//! Debug bar contract tests run through the real in-process HTTP/1 adapter.

mod collection;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
mod gate;
mod integration;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;
mod responses;
mod support;
