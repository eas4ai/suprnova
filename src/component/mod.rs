//! Reconstructible owned component instances and deterministic lifecycle execution.

pub mod composition;
mod executor;
mod instance;
pub mod lazy;
mod lifecycle;

pub use executor::{ComponentExecutor, LifecycleOutput};
pub use instance::{
    ComponentError, ComponentErrorKind, ComponentFactory, ComponentHooks, ComponentInstance,
    HydrationContext, LiveFuture, MountContext, RenderContext,
};
pub use lifecycle::{LifecycleError, LifecycleErrorKind, LifecyclePhase};
