//! Immutable application component registration.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::__private::ComponentRegistration;
use suprnova_live::identity::ComponentName;
use suprnova_live::validation::ValidationPort;

mod sealed {
    pub trait Sealed {}

    impl<T> Sealed for T where T: suprnova_live::metadata::LiveComponentContract {}
}

/// Contract implemented automatically for a macro-produced Live component.
///
/// Applications normally use this trait only as a generic bound. The Live
/// macros own its hidden registration method and keep engine descriptors out of
/// application code.
#[allow(
    private_bounds,
    reason = "the private supertrait seals registration to generated engine contracts"
)]
pub trait ComponentContract: sealed::Sealed {
    /// Produces one validated generated registration for startup insertion.
    #[doc(hidden)]
    fn __live_registration() -> Result<ComponentRegistration, RegistryError>;
}

impl<T> ComponentContract for T
where
    T: suprnova_live::metadata::LiveComponentContract,
{
    fn __live_registration() -> Result<ComponentRegistration, RegistryError> {
        let descriptor = <T as suprnova_live::metadata::LiveComponentContract>::descriptor()
            .map_err(|_| RegistryError::new(RegistryErrorKind::InvalidComponent))?;
        let mut registration = ComponentRegistration::new(descriptor);
        if let Some(validation) =
            <T as suprnova_live::metadata::LiveComponentContract>::validation_port()
        {
            registration = registration.with_validation(validation);
        }
        Ok(registration)
    }
}

/// Immutable process-local component registry built before the server accepts traffic.
#[derive(Clone)]
pub struct LiveRegistry {
    inner: Arc<LiveRegistryGraph>,
}

struct LiveRegistryGraph {
    engine: suprnova_live::registry::ComponentRegistry,
    validation: BTreeMap<ComponentName, Arc<dyn ValidationPort>>,
}

impl LiveRegistry {
    /// Starts an empty bounded registry builder.
    #[must_use]
    pub const fn builder() -> LiveRegistryBuilder {
        LiveRegistryBuilder::new()
    }

    /// Returns the number of explicitly registered component contracts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.engine.len()
    }

    /// Returns whether no component contract was registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.engine.is_empty()
    }

    /// Returns every registered component name in sorted order.
    pub(crate) fn component_names(&self) -> Vec<ComponentName> {
        self.inner.engine.names().cloned().collect()
    }

    pub(crate) fn engine(&self) -> &suprnova_live::registry::ComponentRegistry {
        &self.inner.engine
    }

    pub(crate) fn validation(&self, component: &ComponentName) -> Option<&Arc<dyn ValidationPort>> {
        self.inner.validation.get(component)
    }

    pub(crate) fn from_engine(inner: suprnova_live::registry::ComponentRegistry) -> Self {
        Self {
            inner: Arc::new(LiveRegistryGraph {
                engine: inner,
                validation: BTreeMap::new(),
            }),
        }
    }
}

impl fmt::Debug for LiveRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveRegistry")
            .field("components", &self.len())
            .finish_non_exhaustive()
    }
}

/// Startup-only builder consumed into an immutable [`LiveRegistry`].
pub struct LiveRegistryBuilder {
    inner: suprnova_live::registry::ComponentRegistryBuilder,
    validation: BTreeMap<ComponentName, Arc<dyn ValidationPort>>,
}

impl LiveRegistryBuilder {
    /// Creates an empty explicit component registry builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: suprnova_live::registry::ComponentRegistryBuilder::new(),
            validation: BTreeMap::new(),
        }
    }

    /// Registers one macro-produced component contract.
    pub fn register<C: ComponentContract>(self) -> Result<Self, RegistryError> {
        let registration = C::__live_registration()?;
        self.register_registration(registration)
    }

    pub(crate) fn register_registration(
        mut self,
        registration: ComponentRegistration,
    ) -> Result<Self, RegistryError> {
        let (descriptor, validation) = registration.into_parts();
        let component = descriptor.metadata().identity().clone();
        let requires_validation = descriptor.metadata().actions().iter().any(|action| {
            !matches!(
                action.validation(),
                suprnova_live::validation::ValidationSelection::None
            )
        });
        if requires_validation && validation.is_none() {
            return Err(RegistryError::new(RegistryErrorKind::InvalidComponent));
        }
        self.inner = self.inner.register(descriptor).map_err(|error| {
            let kind = match error.kind() {
                suprnova_live::registry::RegistryErrorKind::DuplicateComponent => {
                    RegistryErrorKind::DuplicateComponent
                }
                suprnova_live::registry::RegistryErrorKind::DuplicateView => {
                    RegistryErrorKind::DuplicateView
                }
                suprnova_live::registry::RegistryErrorKind::CapacityExceeded => {
                    RegistryErrorKind::CapacityExceeded
                }
                suprnova_live::registry::RegistryErrorKind::NotRegistered
                | suprnova_live::registry::RegistryErrorKind::ContractMismatch => {
                    RegistryErrorKind::InvalidComponent
                }
            };
            RegistryError::new(kind)
        })?;
        if let Some(validation) = validation {
            let previous = self.validation.insert(component, validation);
            debug_assert!(
                previous.is_none(),
                "engine registration rejects duplicate names"
            );
        }
        Ok(self)
    }

    /// Consumes startup state into the immutable process registry.
    #[must_use]
    pub fn build(self) -> LiveRegistry {
        LiveRegistry {
            inner: Arc::new(LiveRegistryGraph {
                engine: self.inner.build(),
                validation: self.validation,
            }),
        }
    }
}

impl Default for LiveRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LiveRegistryBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveRegistryBuilder:redacted>")
    }
}

/// Closed reason explicit component registration failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RegistryErrorKind {
    /// Generated metadata was invalid or incompatible with this framework version.
    InvalidComponent,
    /// Two descriptors claimed the same component identity.
    DuplicateComponent,
    /// Two descriptors claimed the same checked root view.
    DuplicateView,
    /// Startup registration exceeded the hard component-count bound.
    CapacityExceeded,
}

impl RegistryErrorKind {
    /// Returns the stable machine-readable failure value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidComponent => "invalid_live_component",
            Self::DuplicateComponent => "duplicate_live_component",
            Self::DuplicateView => "duplicate_live_component_view",
            Self::CapacityExceeded => "live_component_capacity_exceeded",
        }
    }
}

/// Redacted component-registration failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RegistryError {
    kind: RegistryErrorKind,
}

impl RegistryError {
    const fn new(kind: RegistryErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed registration failure category.
    #[must_use]
    pub const fn kind(self) -> RegistryErrorKind {
        self.kind
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for RegistryError {}
