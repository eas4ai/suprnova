//! Internal engine for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Bounded RFC 8785-compatible values and codecs.
pub mod canonical;
/// Injectable wall-clock boundary for validity and lease decisions.
pub mod clock;
/// Reconstructible owned component instances and deterministic lifecycle execution.
pub mod component;
/// Shared Rust/TypeScript golden fixture catalog.
pub mod conformance;
/// Purpose-separated snapshot key derivation and integrity proofs.
pub mod crypto;
/// Stable error categories and safe recovery instructions.
pub mod error;
/// Typed host authority, mount catalog, and request capability contracts.
pub mod host;
/// Validated protocol and snapshot identity types.
pub mod identity;
/// Tier-independent instance revision authority and the complete Tier 0 provider.
pub mod ledger;
/// Resource limits applied at external boundaries.
pub mod limits;
/// Canonical generated component, field, action, and version metadata.
pub mod metadata;
/// Atomic identity-bound initial mounting and inert browser metadata.
pub mod mount;
/// Bounded public-seed promotion into scoped instance authority.
pub mod promotion;
/// Versioned bounded Live control protocol and response ordering model.
pub mod protocol;
/// Server-controlled instance identity generation.
pub mod random;
/// Explicit immutable component registration and contract lookup.
pub mod registry;
/// Versioned signed snapshots and verified hydration capabilities.
pub mod snapshot;
/// Typed state codecs, proposal authorization, and host-neutral binding metadata.
pub mod state;
/// Closed low-cardinality observability labels.
pub mod telemetry;
pub mod view;

/// Version of the internal Live engine crate.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot schema versions understood by this engine.
pub const SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1];

/// Wire protocol versions understood by this engine.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1, 2];
