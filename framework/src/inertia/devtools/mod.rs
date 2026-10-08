//! Server support for the Inertia DevTools browser extension:
//! inertia-laravel's `Inertia\DevTools` namespace.
//!
//! With DevTools on, [`DevToolsMiddleware`] records every request as an
//! entry: the request and response, the kind of visit, the component a
//! page rendered and where in the application's code it was rendered, and
//! for each prop what kind it is and where a shared one was shared. The
//! entry is stored as one JSON file, the response carries its id in
//! `X-Inertia-Devtools-Id`, and the extension fetches it from
//! `GET /_inertia/devtools/entries/{id}`.
//!
//! - `config` - [`DevToolsConfig`], `InertiaConfig::devtools`.
//! - `middleware` - recording and the response headers.
//! - `endpoints` - the two endpoints and their authorization.
//! - `entry` - the entry's JSON, key for key Laravel's.
//! - `recorder`, `classify`, `source` - what a page render records.
//! - `store`, `redact`, `ulid` - storage, redaction and ids.

mod classify;
mod config;
mod endpoints;
mod entry;
mod middleware;
mod recorder;
mod redact;
mod source;
mod store;
mod ulid;

pub(crate) use classify::{ClassifyRequest, classify, errors_meta};
pub use config::DevToolsConfig;
pub use middleware::DevToolsMiddleware;
pub(crate) use middleware::RESPONSE_HEADERS as DEVTOOLS_RESPONSE_HEADERS;
pub(crate) use recorder::{Collector, MultipartReport, current as current_recorder};
pub(crate) use source::SourceLocation;
