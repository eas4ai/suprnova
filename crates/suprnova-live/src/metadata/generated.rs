//! Stable support traits implemented by generated component code.

use crate::component::ComponentHooks;
use crate::registry::ComponentDescriptor;

use super::{ActionMetadata, ComponentMetadata, MetadataError};

/// Generated state-side metadata used to assemble a complete component contract.
#[doc(hidden)]
pub trait LiveComponentDefinitionMetadata {
    /// Builds canonical metadata with the actions discovered on the Live impl.
    fn component_metadata(actions: Vec<ActionMetadata>)
    -> Result<ComponentMetadata, MetadataError>;
}

/// Explicit startup descriptor generated for a complete Live component.
pub trait LiveComponentContract {
    /// Builds the canonical descriptor for explicit registry insertion.
    fn descriptor() -> Result<ComponentDescriptor, MetadataError>;

    /// Builds the same canonical descriptor with generated runtime hooks attached.
    fn descriptor_with_hooks(hooks: ComponentHooks) -> Result<ComponentDescriptor, MetadataError>;

    /// Returns the generated component-specific validation callback when declared.
    fn validation_port() -> Option<std::sync::Arc<dyn crate::validation::ValidationPort>> {
        None
    }
}
