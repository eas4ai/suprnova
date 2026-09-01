//! Reconstructible owned component instances and deterministic lifecycle execution.

pub mod composition;
mod executor;
#[doc(hidden)]
pub mod generated;
mod instance;
pub mod lazy;
mod lifecycle;

pub(crate) use executor::ActionExecutionParts;
pub use executor::{
    ActionExecutionError, ActionExecutionErrorKind, ActionExecutionOutput, ComponentExecutor,
    LifecycleOutput,
};
pub use instance::{
    ComponentError, ComponentErrorKind, ComponentFactory, ComponentHooks, ComponentInstance,
    HydrationContext, LiveFuture, MountContext, RenderContext,
};
pub use lifecycle::{LifecycleError, LifecycleErrorKind, LifecyclePhase};
