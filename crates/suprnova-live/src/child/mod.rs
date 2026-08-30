//! Signed, bounded parent-to-child parameter capabilities.

mod codec;
mod schema;
mod verified;

pub use codec::verify_child_parameters;
pub use schema::{
    CHILD_PARAMETERS_SCHEMA_V1, ChildParameterError, ChildParameterErrorKind, ChildParameterLimits,
    ChildParametersV1, ExpectedChildParametersV1, PreparedChildParametersV1,
};
pub use verified::{AcceptedParentRevision, VerifiedChildParametersV1};
