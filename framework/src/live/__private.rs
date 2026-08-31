//! Generated-code ABI. Application code must not use this module directly.

#![allow(
    missing_docs,
    reason = "the generated-code ABI is intentionally hidden from application documentation"
)]

/// Opaque bridge from generated component metadata into the public registry.
#[doc(hidden)]
pub struct ComponentRegistration(suprnova_live::registry::ComponentDescriptor);

impl ComponentRegistration {
    pub(crate) const fn new(descriptor: suprnova_live::registry::ComponentDescriptor) -> Self {
        Self(descriptor)
    }

    pub(crate) fn into_engine(self) -> suprnova_live::registry::ComponentDescriptor {
        self.0
    }
}

#[doc(hidden)]
pub mod action {
    pub use suprnova_live::action::ActionArgumentField;
    pub use suprnova_live::action::ActionArgumentSchema;
    pub use suprnova_live::action::ActionEntry;
    pub use suprnova_live::action::ActionError;
    pub use suprnova_live::action::ActionTable;
    pub use suprnova_live::action::AuthorizationRequirement;
    pub use suprnova_live::action::AuthorizedAction;
    pub use suprnova_live::action::IntoActionResult;
    pub use suprnova_live::action::TransactionPolicy;
}

#[doc(hidden)]
pub mod component {
    pub use suprnova_live::component::ComponentHooks;

    #[doc(hidden)]
    pub mod composition {
        pub use suprnova_live::component::composition::ChildParameterField;
        pub use suprnova_live::component::composition::ChildParameterSchema;
    }
}

#[doc(hidden)]
pub mod identity {
    pub use suprnova_live::identity::ActionName;
    pub use suprnova_live::identity::ComponentName;
    pub use suprnova_live::identity::ModelField;
    pub use suprnova_live::identity::ViewName;
}

#[doc(hidden)]
pub mod metadata {
    pub use suprnova_live::metadata::ActionMetadata;
    pub use suprnova_live::metadata::ComponentMetadata;
    pub use suprnova_live::metadata::ContractVersions;
    pub use suprnova_live::metadata::EffectMetadata;
    pub use suprnova_live::metadata::EffectPayloadMetadata;
    pub use suprnova_live::metadata::EventMetadata;
    pub use suprnova_live::metadata::EventPayloadMetadata;
    pub use suprnova_live::metadata::FieldMetadata;
    pub use suprnova_live::metadata::LiveComponentContract;
    pub use suprnova_live::metadata::LiveComponentDefinitionMetadata;
    pub use suprnova_live::metadata::MetadataError;
}

#[doc(hidden)]
pub mod registry {
    pub use suprnova_live::registry::ComponentDescriptor;
}

#[doc(hidden)]
pub mod snapshot {
    #[doc(hidden)]
    pub mod state {
        pub use suprnova_live::snapshot::state::FieldCategory;
        pub use suprnova_live::snapshot::state::StateCodec;
    }
}

#[doc(hidden)]
pub mod state {
    pub use suprnova_live::state::BindingTiming;
    pub use suprnova_live::state::ModelCodec;
    pub use suprnova_live::state::UrlBinding;
    pub use suprnova_live::state::UrlBindingMode;
}

#[doc(hidden)]
pub mod validation {
    pub use suprnova_live::validation::ValidationSelection;
}
