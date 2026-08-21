//! Bounded first-action promotion from reusable public seeds to scoped instances.

mod context;
mod error;
mod policy;
mod service;

pub use context::{PromotionAttestations, TrustedPromotionContext};
pub use error::{PromotionError, PromotionErrorKind};
pub use policy::{PromotionLimitConfig, PromotionLimits};
pub use service::{PromotedInstance, PromotionService, RefreshBeforeAction};

pub use crate::random::{
    InstanceIdGenerator, RandomError, RandomErrorKind, SystemInstanceIdGenerator,
};
