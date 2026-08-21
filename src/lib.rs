//! Internal engine for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Bounded RFC 8785-compatible values and codecs.
pub mod canonical;
/// Injectable wall-clock boundary for validity and lease decisions.
pub mod clock;
/// Purpose-separated snapshot key derivation and integrity proofs.
pub mod crypto;
/// Stable error categories and safe recovery instructions.
pub mod error;
/// Validated protocol and snapshot identity types.
pub mod identity;
/// Tier-independent instance revision authority and the complete Tier 0 provider.
pub mod ledger;
/// Resource limits applied at external boundaries.
pub mod limits;
/// Bounded public-seed promotion into scoped instance authority.
pub mod promotion;
/// Versioned bounded Live control protocol and response ordering model.
pub mod protocol;
/// Server-controlled instance identity generation.
pub mod random;
/// Versioned signed snapshots and verified hydration capabilities.
pub mod snapshot;

/// Version of the internal Live engine crate.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot schema versions understood by this engine.
pub const SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1];

/// Wire protocol versions understood by this engine.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1];
