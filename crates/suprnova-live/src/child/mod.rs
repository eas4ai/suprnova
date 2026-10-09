//! Signed, bounded parent-to-child parameter capabilities.

mod codec;
mod eligibility;
mod schema;
mod verified;

pub use codec::{verify_child_parameters, verify_child_parameters_v2};
pub use eligibility::{
    ChildParameterEligibilityError, ChildParameterEligibilityErrorKind, EligibleChildParametersV2,
    authorize_child_parameters_v2,
};
pub use schema::{
    CHILD_PARAMETERS_SCHEMA_V1, CHILD_PARAMETERS_SCHEMA_V2, ChildParameterError,
    ChildParameterErrorKind, ChildParameterLimits, ChildParametersV1, ChildParametersV2,
    ExpectedChildParametersV1, ExpectedChildParametersV2, PreparedChildParametersV1,
    PreparedChildParametersV2,
};
pub use verified::{AcceptedParentRevision, VerifiedChildParametersV1, VerifiedChildParametersV2};
