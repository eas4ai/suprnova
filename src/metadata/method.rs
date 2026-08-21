//! Registered component-action metadata.

use crate::identity::ActionName;

use super::{MetadataError, MetadataErrorKind};

/// Canonical metadata for one explicitly registered Live action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionMetadata {
    name: ActionName,
    version: u16,
}

impl ActionMetadata {
    /// Creates one independently versioned action entry.
    pub fn new(name: ActionName, version: u16) -> Result<Self, MetadataError> {
        if version == 0 {
            return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
        }
        Ok(Self { name, version })
    }

    /// Returns the registered action identity.
    #[must_use]
    pub const fn name(&self) -> &ActionName {
        &self.name
    }

    /// Returns the action's independent argument/behavior version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
}
