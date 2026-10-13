//! Atomic identity-bound initial mounting and inert browser metadata.

mod error;
mod failure;
mod output;
mod public;
mod service;

pub use error::{MountError, MountErrorKind};
pub use failure::MountFailure;
pub use output::{
    DocumentMountKey, DocumentMountScope, MountFlags, PrivateMountOutput, PrivateMountRequest,
};
pub use public::{
    PublicMountProviders, PublicSeedMountOutput, PublicSeedMountRequest, PublicSeedMountService,
};
pub use service::{MountLimits, MountProviders, PrivateMountService};
