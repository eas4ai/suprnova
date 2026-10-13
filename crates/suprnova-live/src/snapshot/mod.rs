//! Versioned seed and instanced snapshots with verify-before-hydration capabilities.

mod codec;
mod composition;
mod error;
mod limits;
mod schema;
/// Registered state schemas and lossless tagged codecs.
pub mod state;
mod verified;

pub(crate) use codec::inspect_instance_authority;
pub use codec::{verify_instance, verify_seed};
pub use composition::{
    COMPOSITION_LINEAGE_EXTENSION_V1, CompositionChildLineageV1, CompositionLineageV1,
    CompositionOwnerLineageV1, MAX_COMPOSITION_LINEAGE_BYTES_V1,
    MAX_COMPOSITION_LINEAGE_CHILDREN_V1, MAX_COMPOSITION_LINEAGE_DEPTH_V1,
};
pub use error::{SnapshotError, SnapshotErrorKind};
pub use limits::SnapshotLimits;
pub(crate) use schema::mounted_document_path;
pub use schema::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, GenerationMemo, InstanceBodyV1,
    InstanceFieldsV1, MountedDocumentPath, SNAPSHOT_SCHEMA_V1, SeedBodyV1, SeedFieldsV1,
    SnapshotForm,
};
pub use state::SnapshotSchemaSet;
pub use verified::{VerifiedInstanceV1, VerifiedSeedV1};
