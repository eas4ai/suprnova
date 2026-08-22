//! Object-safe component instance and generated factory boundaries.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::canonical::CanonicalValue;
use crate::child::VerifiedChildParametersV1;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{InstanceId, Revision, UnixMillis};
use crate::metadata::ComponentMetadata;
use crate::snapshot::state::StateExposure;
use crate::view::IslandRender;

/// Bounded boxed future used by generated object-safe component hooks.
pub type LiveFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Closed component-supplied failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComponentErrorKind {
    /// Application component code rejected the current lifecycle phase.
    ApplicationFailure,
    /// Generated state or view code could not satisfy its declared contract.
    ContractFailure,
}

/// Redacted error returned by application or generated component code.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ComponentError {
    kind: ComponentErrorKind,
}

impl ComponentError {
    /// Creates a redacted application lifecycle failure.
    #[must_use]
    pub const fn application_failure() -> Self {
        Self {
            kind: ComponentErrorKind::ApplicationFailure,
        }
    }

    /// Creates a redacted generated-contract failure.
    #[must_use]
    pub const fn contract_failure() -> Self {
        Self {
            kind: ComponentErrorKind::ContractFailure,
        }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> ComponentErrorKind {
        self.kind
    }
}

impl fmt::Display for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ComponentErrorKind::ApplicationFailure => "component_application_failure",
            ComponentErrorKind::ContractFailure => "component_contract_failure",
        })
    }
}

impl fmt::Debug for ComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ComponentError {}

/// Immutable request facts visible while rendering one owned component instance.
#[derive(Clone, Copy)]
pub struct RenderContext<'a> {
    request: &'a TrustedLiveRequestContext,
    instance_id: &'a InstanceId,
    revision: Revision,
    expires_at: UnixMillis,
}

impl<'a> RenderContext<'a> {
    /// Binds component execution to validated host authority and server identity.
    #[must_use]
    pub const fn new(
        request: &'a TrustedLiveRequestContext,
        instance_id: &'a InstanceId,
        revision: Revision,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            request,
            instance_id,
            revision,
            expires_at,
        }
    }

    /// Returns the validated host request capability.
    #[must_use]
    pub const fn request(&self) -> &TrustedLiveRequestContext {
        self.request
    }

    /// Returns the server-assigned component instance identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.instance_id
    }

    /// Returns the revision being rendered.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the exclusive instance expiration deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }
}

impl fmt::Debug for RenderContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<RenderContext:redacted>")
    }
}

/// Trusted context and explicit parameters for a repeatable initial mount.
#[derive(Clone, Copy)]
pub struct MountContext<'a> {
    render: RenderContext<'a>,
    parameters: &'a CanonicalValue,
}

impl<'a> MountContext<'a> {
    /// Creates mount input from trusted request facts and typed canonical parameters.
    #[must_use]
    pub const fn new(render: RenderContext<'a>, parameters: &'a CanonicalValue) -> Self {
        Self { render, parameters }
    }

    /// Returns the render context bound to the proposed identity.
    #[must_use]
    pub const fn render(&self) -> &RenderContext<'a> {
        &self.render
    }

    /// Returns the validated explicit mount parameters.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        self.parameters
    }
}

impl fmt::Debug for MountContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<MountContext:redacted>")
    }
}

/// Verified state input used to reconstruct a fresh owned instance for a request.
#[derive(Clone, Copy)]
pub struct HydrationContext<'a> {
    render: RenderContext<'a>,
    state: &'a CanonicalValue,
}

impl<'a> HydrationContext<'a> {
    /// Creates reconstruction input from trusted request facts and verified state.
    #[must_use]
    pub const fn new(render: RenderContext<'a>, state: &'a CanonicalValue) -> Self {
        Self { render, state }
    }

    /// Returns the render context bound to this reconstruction.
    #[must_use]
    pub const fn render(&self) -> &RenderContext<'a> {
        &self.render
    }

    /// Returns verified canonical component state.
    #[must_use]
    pub const fn state(&self) -> &CanonicalValue {
        self.state
    }
}

impl fmt::Debug for HydrationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<HydrationContext:redacted>")
    }
}

/// One request-owned component object. It is never retained by the engine.
pub trait ComponentInstance: Send {
    /// Returns the generated component contract implemented by this object.
    fn metadata(&self) -> &'static ComponentMetadata;

    /// Runs after verified state created this fresh request-owned object.
    fn hydrated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Applies one separately verified child parameter capability before rendering.
    fn params_changed<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _parameters: &'a VerifiedChildParametersV1,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Err(ComponentError::contract_failure()) })
    }

    /// Completes deferred server work through the ordinary render lifecycle.
    fn lazy_complete<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Err(ComponentError::contract_failure()) })
    }

    /// Runs at the final async mutation point before immutable rendering.
    fn rendering<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Produces bounded island data without response authority.
    fn render<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>>;

    /// Runs after rendering and before dehydration begins.
    fn rendered<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Runs at the final async mutation point before immutable dehydration.
    fn dehydrating<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Deterministically serializes state eligible for the requested exposure.
    fn dehydrate(&self, exposure: StateExposure) -> Result<CanonicalValue, ComponentError>;

    /// Deterministically serializes lifecycle memo; the default carries no memo.
    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Null)
    }

    /// Releases request-owned resources exactly once after successful construction.
    fn teardown<'a>(&'a mut self) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Generated owned-instance creation and verified reconstruction hooks.
pub trait ComponentFactory: Send + Sync {
    /// Creates one component under a candidate server identity.
    ///
    /// This initializer must be repeatable and effect-free because a private
    /// mount may discard the complete result and retry after an identity
    /// collision. Durable domain work belongs in later accepted actions.
    fn mount<'a>(
        &'a self,
        context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>>;

    /// Reconstructs one fresh component from verified canonical state.
    fn hydrate<'a>(
        &'a self,
        context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>>;
}

/// Cloneable descriptor-owned generated component hooks.
#[derive(Clone)]
pub struct ComponentHooks {
    factory: Arc<dyn ComponentFactory>,
}

impl ComponentHooks {
    /// Erases one generated concrete component factory for registry storage.
    #[must_use]
    pub fn new(factory: Arc<dyn ComponentFactory>) -> Self {
        Self { factory }
    }

    pub(crate) fn factory(&self) -> &dyn ComponentFactory {
        self.factory.as_ref()
    }
}

impl fmt::Debug for ComponentHooks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ComponentHooks:generated>")
    }
}
