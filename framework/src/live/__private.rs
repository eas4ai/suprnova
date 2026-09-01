//! Generated-code ABI. Application code must not use this module directly.

#![allow(
    missing_docs,
    reason = "the generated-code ABI is intentionally hidden from application documentation"
)]

/// Pinned checked-template runtime used only by generated view code.
#[doc(hidden)]
pub use askama;

/// Opaque bridge from generated component metadata into the public registry.
#[doc(hidden)]
pub struct ComponentRegistration {
    descriptor: suprnova_live::registry::ComponentDescriptor,
    validation: Option<std::sync::Arc<dyn suprnova_live::validation::ValidationPort>>,
}

impl ComponentRegistration {
    pub(crate) const fn new(descriptor: suprnova_live::registry::ComponentDescriptor) -> Self {
        Self {
            descriptor,
            validation: None,
        }
    }

    pub(crate) fn with_validation(
        mut self,
        validation: std::sync::Arc<dyn suprnova_live::validation::ValidationPort>,
    ) -> Self {
        self.validation = Some(validation);
        self
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        suprnova_live::registry::ComponentDescriptor,
        Option<std::sync::Arc<dyn suprnova_live::validation::ValidationPort>>,
    ) {
        (self.descriptor, self.validation)
    }

    pub(crate) fn into_engine(self) -> suprnova_live::registry::ComponentDescriptor {
        self.descriptor
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
    pub use suprnova_live::validation::{
        ValidationFuture, ValidationIssue, ValidationPort, ValidationPortError, ValidationRequest,
        ValidationSelection,
    };

    pub trait IntoValidationIssues {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError>;
    }

    pub fn into_validation_issues<T: IntoValidationIssues>(
        output: T,
    ) -> Result<Vec<ValidationIssue>, ValidationPortError> {
        output.into_validation_issues()
    }

    impl IntoValidationIssues for () {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError> {
            Ok(Vec::new())
        }
    }

    impl IntoValidationIssues for Vec<ValidationIssue> {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError> {
            Ok(self)
        }
    }

    impl IntoValidationIssues for Result<Vec<ValidationIssue>, ValidationPortError> {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError> {
            self
        }
    }

    impl IntoValidationIssues for validator::ValidationErrors {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError> {
            validator_issues(&self)
        }
    }

    impl IntoValidationIssues for Result<(), validator::ValidationErrors> {
        fn into_validation_issues(self) -> Result<Vec<ValidationIssue>, ValidationPortError> {
            match self {
                Ok(()) => Ok(Vec::new()),
                Err(errors) => validator_issues(&errors),
            }
        }
    }

    fn validator_issues(
        errors: &validator::ValidationErrors,
    ) -> Result<Vec<ValidationIssue>, ValidationPortError> {
        let mut issues = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        collect_validator_issues(errors, "", &mut issues, &mut seen)?;
        Ok(issues)
    }

    fn collect_validator_issues(
        errors: &validator::ValidationErrors,
        prefix: &str,
        issues: &mut Vec<ValidationIssue>,
        seen: &mut std::collections::BTreeSet<(String, String)>,
    ) -> Result<(), ValidationPortError> {
        for (field, kind) in errors.errors() {
            let path = if prefix.is_empty() {
                field.to_string()
            } else {
                format!("{prefix}.{field}")
            };
            match kind {
                validator::ValidationErrorsKind::Field(field_errors) => {
                    for error in field_errors {
                        let message = format!("validation.{}", error.code);
                        if seen.insert((path.clone(), message.clone())) {
                            issues.push(ValidationIssue::new(
                                suprnova_live::state::ModelPath::parse(&path)
                                    .map_err(|_| ValidationPortError::unavailable())?,
                                suprnova_live::validation::ValidationMessageId::parse(&message)
                                    .map_err(|_| ValidationPortError::unavailable())?,
                            ));
                        }
                    }
                }
                validator::ValidationErrorsKind::Struct(inner) => {
                    collect_validator_issues(inner, &path, issues, seen)?;
                }
                validator::ValidationErrorsKind::List(items) => {
                    for inner in items.values() {
                        // Browser indices are positional and therefore not
                        // stable Live model-path segments. Collapse them while
                        // preserving collection/nested field names. Multiple
                        // elements can therefore normalize to the same issue;
                        // first-seen deduplication keeps engine limits semantic.
                        collect_validator_issues(inner, &path, issues, seen)?;
                    }
                }
            }
        }
        Ok(())
    }
}
