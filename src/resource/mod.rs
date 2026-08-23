//! Executor-neutral bounded resource ownership and lifecycle primitives.

mod bounds;
mod cancel;
mod owner;
mod queue;

pub use bounds::{
    HARD_MAX_ACTIVE_PERMITS, HARD_MAX_RESOURCE_BYTES, HARD_MAX_RESOURCE_ITEMS, ResourceBounds,
    ResourceBoundsError,
};
pub use cancel::CancellationFlag;
pub use owner::{Permit, PermitPool, ResourceOwner, ResourceQueue};
pub use queue::{BoundedQueue, ResourceDiagnostic, ResourceError, Retirement};
