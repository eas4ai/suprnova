//! One explicitly registered component descriptor.

use std::fmt;

use crate::action::{ActionError, ActionTable};
use crate::component::ComponentHooks;
use crate::component::composition::ChildParameterSchema;
use crate::identity::ContentDigest;
use crate::metadata::ComponentMetadata;

/// Runtime descriptor generated for one component contract.
#[derive(Clone)]
pub struct ComponentDescriptor {
    metadata: ComponentMetadata,
    hooks: Option<ComponentHooks>,
    parameter_schema: ChildParameterSchema,
    actions: ActionTable,
    params_changed: bool,
    lazy_complete: bool,
}

impl ComponentDescriptor {
    /// Creates a descriptor from completely validated canonical metadata.
    #[must_use]
    pub fn new(metadata: ComponentMetadata) -> Self {
        Self {
            metadata,
            hooks: None,
            parameter_schema: ChildParameterSchema::default(),
            actions: ActionTable::default(),
            params_changed: false,
            lazy_complete: false,
        }
    }

    /// Creates an executable descriptor with generated owned-instance hooks.
    #[must_use]
    pub fn with_hooks(metadata: ComponentMetadata, hooks: ComponentHooks) -> Self {
        Self {
            metadata,
            hooks: Some(hooks),
            parameter_schema: ChildParameterSchema::default(),
            actions: ActionTable::default(),
            params_changed: false,
            lazy_complete: false,
        }
    }

    /// Attaches generated parameter and closed lifecycle-operation contracts.
    #[must_use]
    pub fn with_composition(
        mut self,
        parameter_schema: ChildParameterSchema,
        params_changed: bool,
        lazy_complete: bool,
    ) -> Self {
        self.parameter_schema = parameter_schema;
        self.params_changed = params_changed;
        self.lazy_complete = lazy_complete;
        self
    }

    /// Attaches the generated exact-method action table after metadata equivalence validation.
    pub fn with_actions(mut self, actions: ActionTable) -> Result<Self, ActionError> {
        if !actions.matches_metadata(self.metadata.actions()) {
            return Err(ActionError::dispatcher_contract());
        }
        self.actions = actions;
        Ok(self)
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

    /// Returns the generated typed mount-parameter contract.
    #[must_use]
    pub const fn parameter_schema(&self) -> &ChildParameterSchema {
        &self.parameter_schema
    }

    /// Returns the closed generated action table.
    #[must_use]
    pub const fn actions(&self) -> &ActionTable {
        &self.actions
    }

    /// Returns whether the closed `params_changed` operation is registered.
    #[must_use]
    pub const fn supports_params_changed(&self) -> bool {
        self.params_changed
    }

    /// Returns whether the closed `lazy_complete` operation is registered.
    #[must_use]
    pub const fn supports_lazy_complete(&self) -> bool {
        self.lazy_complete
    }
}

impl PartialEq for ComponentDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.metadata == other.metadata
            && self.parameter_schema == other.parameter_schema
            && self.params_changed == other.params_changed
            && self.lazy_complete == other.lazy_complete
    }
}

impl Eq for ComponentDescriptor {}

impl fmt::Debug for ComponentDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentDescriptor")
            .field("metadata", &self.metadata)
            .field("executable", &self.hooks.is_some())
            .field("action_dispatchers", &self.actions.len())
            .field("params_changed", &self.params_changed)
            .field("lazy_complete", &self.lazy_complete)
            .finish()
    }
}
