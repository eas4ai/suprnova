//! Registered component-action metadata.

use crate::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
use crate::identity::ActionName;
use crate::validation::ValidationSelection;

use super::{MetadataError, MetadataErrorKind};

/// Canonical metadata for one explicitly registered Live action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionMetadata {
    name: ActionName,
    version: u16,
    arguments: ActionArgumentSchema,
    authorization: AuthorizationRequirement,
    validation: ValidationSelection,
    transaction: TransactionPolicy,
}

impl ActionMetadata {
    /// Creates one independently versioned action entry.
    pub fn new(name: ActionName, version: u16) -> Result<Self, MetadataError> {
        if version == 0 {
            return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
        }
        Ok(Self {
            name,
            version,
            arguments: ActionArgumentSchema::empty(),
            authorization: AuthorizationRequirement::Public,
            validation: ValidationSelection::None,
            transaction: TransactionPolicy::None,
        })
    }

    /// Creates one complete generated action dispatch contract.
    pub fn new_with_contract(
        name: ActionName,
        version: u16,
        arguments: ActionArgumentSchema,
        authorization: AuthorizationRequirement,
        mut validation: ValidationSelection,
        transaction: TransactionPolicy,
    ) -> Result<Self, MetadataError> {
        if version == 0 {
            return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
        }
        if let ValidationSelection::Selected(paths) = &mut validation {
            paths.sort();
            if paths.is_empty()
                || paths.len() > 128
                || paths.windows(2).any(|pair| pair[0] == pair[1])
            {
                return Err(MetadataError::new(MetadataErrorKind::InvalidActionMetadata));
            }
        }
        Ok(Self {
            name,
            version,
            arguments,
            authorization,
            validation,
            transaction,
        })
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

    /// Returns the generated bounded typed argument schema.
    #[must_use]
    pub const fn arguments(&self) -> &ActionArgumentSchema {
        &self.arguments
    }

    /// Returns the current authorization requirement.
    #[must_use]
    pub const fn authorization(&self) -> AuthorizationRequirement {
        self.authorization
    }

    /// Returns the exact validation selection for this action.
    #[must_use]
    pub const fn validation(&self) -> &ValidationSelection {
        &self.validation
    }

    /// Returns whether host transaction coordination is requested.
    #[must_use]
    pub const fn transaction(&self) -> TransactionPolicy {
        self.transaction
    }
}
