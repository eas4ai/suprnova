//! One explicitly registered component descriptor.

use crate::identity::ContentDigest;
use crate::metadata::ComponentMetadata;

/// Runtime descriptor generated for one component contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentDescriptor {
    metadata: ComponentMetadata,
}

impl ComponentDescriptor {
    /// Creates a descriptor from completely validated canonical metadata.
    #[must_use]
    pub const fn new(metadata: ComponentMetadata) -> Self {
        Self { metadata }
    }

    /// Returns the complete generated component metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ComponentMetadata {
        &self.metadata
    }

    /// Returns the purpose-specific canonical component contract digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        self.metadata.contract_digest()
    }
}
