//! Internal engine for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Bounded RFC 8785-compatible values and codecs.
pub mod canonical;
/// Stable error categories and safe recovery instructions.
pub mod error;
/// Validated protocol and snapshot identity types.
pub mod identity;
/// Resource limits applied at external boundaries.
pub mod limits;

/// Version of the internal Live engine crate.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot schema versions understood by this engine.
pub const SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1];

/// Wire protocol versions understood by this engine.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1];
