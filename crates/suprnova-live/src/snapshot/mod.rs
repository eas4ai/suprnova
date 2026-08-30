//! Versioned seed and instanced snapshots with verify-before-hydration capabilities.

mod codec;
mod error;
mod limits;
mod schema;
/// Registered state schemas and lossless tagged codecs.
pub mod state;
mod verified;

pub use codec::{verify_instance, verify_seed};
pub use error::{SnapshotError, SnapshotErrorKind};
pub use limits::SnapshotLimits;
pub use schema::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, GenerationMemo, InstanceBodyV1,
    InstanceFieldsV1, SNAPSHOT_SCHEMA_V1, SeedBodyV1, SeedFieldsV1, SnapshotForm,
};
pub use state::SnapshotSchemaSet;
pub use verified::{VerifiedInstanceV1, VerifiedSeedV1};
