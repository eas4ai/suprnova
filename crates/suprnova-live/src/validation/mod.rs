//! Bounded localizable validation kept distinct from model binding failures.

mod engine;
mod error_bag;
mod port;

pub use engine::{ValidationEngine, ValidationEngineError, ValidationEngineErrorKind};
pub(crate) use error_bag::HARD_MAX_VALIDATION_ISSUES;
pub use error_bag::{BagPolicy, ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus};
pub use port::{
    ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest, ValidationSelection,
};
