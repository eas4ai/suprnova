//! Bounded Live v1 request, response, compatibility, and ordering contracts.

mod compatibility;
mod error;
mod limits;
mod ordering;
mod request;
mod response;

pub use compatibility::{CompatibilityDecision, CompatibilityWindow, VersionSet};
pub use error::{ProtocolError, ProtocolErrorKind};
pub use limits::{ProtocolLimitConfig, ProtocolLimits};
pub use ordering::{ApplicationStep, MorphDisposition, application_plan};
pub use request::{Operation, SnapshotInput, UpdateRequest, parse_update_request};
pub use response::{
    Emission, RenderPayload, ResponseOutcome, UpdateResponse, parse_update_response,
};
