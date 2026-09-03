//! Internal engine for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Registered typed actions, current authorization, and semantic outcomes.
pub mod action;
/// Reviewed production browser artifacts embedded with their manifest.
pub mod artifacts;
/// Typed, bounded declarations for authorized asynchronous updates.
pub mod async_updates;
/// Bounded RFC 8785-compatible values and codecs.
pub mod canonical;
pub mod checker;
/// Signed parent-to-child parameter capabilities and typed verification.
pub mod child;
/// Injectable wall-clock boundary for validity and lease decisions.
pub mod clock;
/// Reconstructible owned component instances and deterministic lifecycle execution.
pub mod component;
/// Shared Rust/TypeScript golden fixture catalog.
pub mod conformance;
/// Purpose-separated snapshot key derivation and integrity proofs.
pub mod crypto;
/// Host-neutral Live HTTP admission, authority verification, and response intent.
pub mod endpoint;
/// Stable error categories and safe recovery instructions.
pub mod error;
/// Host-neutral action transaction, acceptance, tracing, and recovery coordination.
pub mod execution;
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
/// Typed Complete representations of canonical documents, their policy,
/// variance, key, entry, storage, generation, coherence, and HTTP contracts.
pub mod render_cache;
/// Executor-neutral bounded queues, permits, cancellation, and lifecycle ownership.
pub mod resource;
/// Versioned signed snapshots and verified hydration capabilities.
pub mod snapshot;
/// Typed state codecs, proposal authorization, and host-neutral binding metadata.
pub mod state;
/// Closed low-cardinality observability labels.
pub mod telemetry;
/// Opaque temporary upload identities and secret transfer capabilities.
pub mod upload;
/// Bounded localizable validation and host-neutral validation ports.
pub mod validation;
pub mod view;

/// Version of the internal Live engine crate.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot schema versions understood by this engine.
pub const SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1];

/// Wire protocol versions understood by this engine.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1, 2];
