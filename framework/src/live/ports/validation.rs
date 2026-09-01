//! Fail-closed bridge for generated Live validation selection.

use suprnova_live::validation::{
    ValidationFuture, ValidationIssue, ValidationPort, ValidationPortError, ValidationRequest,
    ValidationSelection,
};

pub(crate) struct SuprnovaValidationPort {
    registry: super::super::LiveRegistry,
}

impl SuprnovaValidationPort {
    pub(crate) const fn new(registry: super::super::LiveRegistry) -> Self {
        Self { registry }
    }
}

impl ValidationPort for SuprnovaValidationPort {
    fn validate<'a>(
        &'a self,
        request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>> {
        if matches!(request.selection(), ValidationSelection::None) {
            return Box::pin(async { Ok(Vec::new()) });
        }

        let Some(validation) = self.registry.validation(request.component()) else {
            return Box::pin(async { Err(ValidationPortError::unavailable()) });
        };
        validation.validate(request)
    }
}
