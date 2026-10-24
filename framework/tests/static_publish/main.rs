#![cfg(all(feature = "testing", feature = "filesystem"))]

#[cfg(feature = "cdn-cloudflare")]
mod cloudflare;
#[cfg(feature = "cdn-cloudfront")]
mod cloudfront;
mod eligibility;
#[cfg(feature = "media")]
mod images;
mod prune;
mod publication;

mod fallback;
mod regeneration;

mod boot;
#[path = "../support/env_lock.rs"]
mod env_lock;
#[path = "../support/env_snapshot.rs"]
mod env_snapshot;
#[path = "../support/http_wire.rs"]
mod http_wire;
#[path = "../support/own_process.rs"]
#[allow(
    dead_code,
    reason = "shared child-process helper also supports synchronous tests"
)]
mod own_process;
#[path = "../support/own_process_async.rs"]
mod own_process_async;

mod client;

#[cfg(not(feature = "media"))]
mod raster_guard;

mod inventory;

mod timeout;

mod document_css;
