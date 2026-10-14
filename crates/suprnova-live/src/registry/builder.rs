//! Explicit bounded startup registry construction.

use std::collections::BTreeMap;

use crate::identity::{ComponentName, ViewName};

use super::{ComponentDescriptor, ComponentRegistry, RegistryError, RegistryErrorKind};

const MAX_COMPONENTS: usize = 10_000;

/// Mutable startup-only builder consumed into an immutable component registry.
#[derive(Debug, Default)]
pub struct ComponentRegistryBuilder {
    components: BTreeMap<ComponentName, ComponentDescriptor>,
    views: BTreeMap<ViewName, ComponentName>,
}

impl ComponentRegistryBuilder {
    /// Creates an empty explicit startup registry builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            components: BTreeMap::new(),
            views: BTreeMap::new(),
        }
    }

    /// Registers one descriptor or rejects duplicate ownership deterministically.
    pub fn register(mut self, descriptor: ComponentDescriptor) -> Result<Self, RegistryError> {
        if self.components.len() >= MAX_COMPONENTS {
            return Err(RegistryError::new(RegistryErrorKind::CapacityExceeded));
        }
        let component = descriptor.metadata().identity().clone();
        let view = descriptor.metadata().view().clone();
        if self.components.contains_key(&component) {
            return Err(RegistryError::new(RegistryErrorKind::DuplicateComponent));
        }
        if self.views.contains_key(&view) {
            return Err(RegistryError::new(RegistryErrorKind::DuplicateView));
        }
        self.views.insert(view, component.clone());
        self.components.insert(component, descriptor);
        Ok(self)
    }

    /// Consumes startup state into the immutable process registry.
    #[must_use]
    pub fn build(self) -> ComponentRegistry {
        ComponentRegistry::new(self.components)
    }
}
