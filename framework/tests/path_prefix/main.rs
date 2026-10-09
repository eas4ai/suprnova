//! Serving under a path prefix (docs/spec/path-prefix.md, PFX-001 to
//! PFX-013): one binary, one module per area. Each test is named after the
//! requirement whose falsifier it checks, lowercase id first.
//!
//! Every test that drives a request installs process-wide state - the
//! `AppConfig` with `APP_URL` and the trusted proxies, route names, the
//! encryption key - so it runs alone in a child process through
//! `own_process_async::delegate`. The server is a real loopback socket
//! served by `handle_request_with_peer`, because a forwarded header is
//! trusted only from a peer address, which the in-process adapter does not
//! give.

#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[path = "../support/live_dogfood_support/mod.rs"]
mod live_dogfood_support;
#[path = "../support/own_process.rs"]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

pub mod cookies;
pub mod inertia;
pub mod links;
pub mod live;
pub mod manual;
pub mod redirects;
pub mod render_cache;
pub mod root;
pub mod routes;
pub mod support;
pub mod trust;
