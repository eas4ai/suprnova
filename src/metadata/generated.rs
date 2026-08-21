//! Stable support traits implemented by generated component code.

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
}
