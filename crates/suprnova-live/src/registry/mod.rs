//! Explicit immutable component registration and contract lookup.

mod builder;
mod descriptor;
mod error;

use std::collections::BTreeMap;

pub use builder::ComponentRegistryBuilder;
pub use descriptor::ComponentDescriptor;
pub use error::{RegistryError, RegistryErrorKind};

use crate::identity::{ComponentName, ContentDigest};

/// Immutable process-local component registry built before serving requests.
#[derive(Clone, Debug)]
pub struct ComponentRegistry {
    components: BTreeMap<ComponentName, ComponentDescriptor>,
}

impl ComponentRegistry {
    pub(crate) const fn new(components: BTreeMap<ComponentName, ComponentDescriptor>) -> Self {
        Self { components }
    }

    /// Returns every registered component name in sorted order.
    pub fn names(&self) -> impl Iterator<Item = &ComponentName> {
        self.components.keys()
    }

    /// Resolves only a component explicitly registered at startup.
    pub fn resolve(
        &self,
        component: &ComponentName,
    ) -> Result<&ComponentDescriptor, RegistryError> {
        self.components
            .get(component)
            .ok_or_else(|| RegistryError::new(RegistryErrorKind::NotRegistered))
    }

    /// Resolves a component and verifies its current canonical contract digest.
    pub fn require_contract(
        &self,
        component: &ComponentName,
        expected: &ContentDigest,
    ) -> Result<&ComponentDescriptor, RegistryError> {
        let descriptor = self.resolve(component)?;
        if descriptor.contract_digest() != expected {
            return Err(RegistryError::new(RegistryErrorKind::ContractMismatch));
        }
        Ok(descriptor)
    }

    /// Returns the bounded number of registered component contracts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Returns whether no components were registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}
