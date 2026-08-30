//! Unforgeable-in-safe-code capabilities produced only after snapshot verification.

use std::fmt;

use serde::de::DeserializeOwned;

use super::state::{StateExposure, StateSchema, hydrate};
use super::{InstanceBodyV1, SeedBodyV1, SnapshotError};

/// Verified reusable public seed capability.
pub struct VerifiedSeedV1 {
    body: SeedBodyV1,
}

impl VerifiedSeedV1 {
    pub(crate) const fn new(body: SeedBodyV1) -> Self {
        Self { body }
    }

    /// Returns the fully verified typed seed body.
    #[must_use]
    pub const fn body(&self) -> &SeedBodyV1 {
        &self.body
    }

    /// Hydrates state into the caller-selected registered Rust type.
    pub fn hydrate_state<T: DeserializeOwned>(
        &self,
        schema: &StateSchema,
    ) -> Result<T, SnapshotError> {
        hydrate(self.body.state(), schema, StateExposure::PublicSeed)
    }

    /// Hydrates lifecycle memo into the caller-selected registered Rust type.
    pub fn hydrate_memo<T: DeserializeOwned>(
        &self,
        schema: &StateSchema,
    ) -> Result<T, SnapshotError> {
        hydrate(self.body.memo(), schema, StateExposure::PublicSeed)
    }

    /// Hydrates public mount parameters into the caller-selected registered type.
    pub fn hydrate_mount<T: DeserializeOwned>(
        &self,
        schema: &StateSchema,
    ) -> Result<T, SnapshotError> {
        hydrate(self.body.mount(), schema, StateExposure::PublicSeed)
    }
}

impl fmt::Debug for VerifiedSeedV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifiedSeedV1:redacted>")
    }
}

/// Verified scoped instanced snapshot capability.
pub struct VerifiedInstanceV1 {
    body: InstanceBodyV1,
}

impl VerifiedInstanceV1 {
    pub(crate) const fn new(body: InstanceBodyV1) -> Self {
        Self { body }
    }

    /// Returns the fully verified typed instanced body.
    #[must_use]
    pub const fn body(&self) -> &InstanceBodyV1 {
        &self.body
    }

    /// Hydrates state into the caller-selected registered Rust type.
    pub fn hydrate_state<T: DeserializeOwned>(
        &self,
        schema: &StateSchema,
    ) -> Result<T, SnapshotError> {
        hydrate(self.body.state(), schema, StateExposure::Instanced)
    }

    /// Hydrates lifecycle memo into the caller-selected registered Rust type.
    pub fn hydrate_memo<T: DeserializeOwned>(
        &self,
        schema: &StateSchema,
    ) -> Result<T, SnapshotError> {
        hydrate(self.body.memo(), schema, StateExposure::Instanced)
    }
}

impl fmt::Debug for VerifiedInstanceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifiedInstanceV1:redacted>")
    }
}
