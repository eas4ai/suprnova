//! One explicitly registered component descriptor.

use std::fmt;

use crate::component::ComponentHooks;
use crate::identity::ContentDigest;
use crate::metadata::ComponentMetadata;

/// Runtime descriptor generated for one component contract.
#[derive(Clone)]
pub struct ComponentDescriptor {
    metadata: ComponentMetadata,
    hooks: Option<ComponentHooks>,
}

impl ComponentDescriptor {
    /// Creates a descriptor from completely validated canonical metadata.
    #[must_use]
    pub const fn new(metadata: ComponentMetadata) -> Self {
        Self {
            metadata,
            hooks: None,
        }
    }

    /// Creates an executable descriptor with generated owned-instance hooks.
    #[must_use]
    pub const fn with_hooks(metadata: ComponentMetadata, hooks: ComponentHooks) -> Self {
        Self {
            metadata,
            hooks: Some(hooks),
        }
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

    /// Returns generated runtime hooks when this descriptor is executable.
    #[must_use]
    pub const fn hooks(&self) -> Option<&ComponentHooks> {
        self.hooks.as_ref()
    }
}

impl PartialEq for ComponentDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.metadata == other.metadata
    }
}

impl Eq for ComponentDescriptor {}

impl fmt::Debug for ComponentDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentDescriptor")
            .field("metadata", &self.metadata)
            .field("executable", &self.hooks.is_some())
            .finish()
    }
}
