//! Host-neutral Live HTTP admission, authority verification, and response intent.

mod config;
mod error;
mod request;
mod response;
mod service;

pub use config::LiveEndpointConfig;
pub use error::{EndpointError, EndpointErrorKind, EndpointKernelError};
pub use request::{
    LIVE_MEDIA_TYPE_V1, LIVE_MEDIA_TYPE_V2, LiveEndpointRequest, ParsedLiveMediaType,
    RequestCachePolicy,
};
pub(crate) use response::{
    AcceptedRequestBinding, AcceptedRequestSnapshotBinding, AcceptedResponseAuthority,
    AcceptedResponseCandidate, AcceptedResponseSnapshotAuthority, SealedAcceptedResponse,
};
pub use response::{
    AcceptedResponseRequestBinding, AcceptedResponseSealer, EndpointDispatch,
    EndpointNavigationTarget, EndpointNavigationTargetError, EndpointOutcomeKind,
    LiveEndpointResponse,
};
pub use response::{EndpointResponseIntents, dispatch_execution_result};
pub use service::{
    EndpointFuture, EndpointKernel, LiveEndpointService, VerifiedChildAdmissionV2,
    VerifiedEndpointExecutionRequest, VerifiedEndpointRequest, VerifiedEndpointSnapshot,
};
