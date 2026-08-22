//! Atomic identity-bound initial mounting and inert browser metadata.

mod error;
mod output;
mod service;

pub use error::{MountError, MountErrorKind};
pub use output::{
    DocumentMountKey, DocumentMountScope, MountFlags, PrivateMountOutput, PrivateMountRequest,
};
pub use service::{MountLimits, MountProviders, PrivateMountService};
