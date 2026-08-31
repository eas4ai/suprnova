//! Immutable application component registration.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::__private::ComponentRegistration;

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
        <T as suprnova_live::metadata::LiveComponentContract>::descriptor()
            .map(ComponentRegistration::new)
            .map_err(|_| RegistryError::new(RegistryErrorKind::InvalidComponent))
    }
}

/// Immutable process-local component registry built before the server accepts traffic.
#[derive(Clone, Debug)]
pub struct LiveRegistry {
    inner: Arc<suprnova_live::registry::ComponentRegistry>,
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
        self.inner.len()
    }

    /// Returns whether no component contract was registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Startup-only builder consumed into an immutable [`LiveRegistry`].
#[derive(Debug, Default)]
pub struct LiveRegistryBuilder {
    inner: suprnova_live::registry::ComponentRegistryBuilder,
}

impl LiveRegistryBuilder {
    /// Creates an empty explicit component registry builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: suprnova_live::registry::ComponentRegistryBuilder::new(),
        }
    }

    /// Registers one macro-produced component contract.
    pub fn register<C: ComponentContract>(mut self) -> Result<Self, RegistryError> {
        let registration = C::__live_registration()?;
        self.inner = self
            .inner
            .register(registration.into_engine())
            .map_err(|error| {
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
        Ok(self)
    }

    /// Consumes startup state into the immutable process registry.
    #[must_use]
    pub fn build(self) -> LiveRegistry {
        LiveRegistry {
            inner: Arc::new(self.inner.build()),
        }
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
