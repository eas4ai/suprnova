//! Server support for the Inertia DevTools browser extension:
//! inertia-laravel's `Inertia\DevTools` namespace.
//!
//! With DevTools on, [`DevToolsMiddleware`] records every request as an
//! entry: the request and response, the kind of visit, and the component
//! a page rendered and where in the application's code it was rendered.
//! The entry is stored as one JSON file, and the response carries its id
//! in `X-Inertia-Devtools-Id`.
//!
//! - `config` - [`DevToolsConfig`], `InertiaConfig::devtools`.
//! - `middleware` - recording and the response headers.
//! - `entry` - the entry's JSON, key for key Laravel's.
//! - `recorder`, `source` - what a page render records.
//! - `store`, `redact`, `ulid` - storage, redaction and ids.

mod config;
mod entry;
mod middleware;
mod recorder;
mod redact;
mod source;
mod store;
mod ulid;

pub use config::DevToolsConfig;
pub use middleware::DevToolsMiddleware;
pub(crate) use recorder::{Collector, current as current_recorder};
pub(crate) use source::SourceLocation;
