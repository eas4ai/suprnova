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
pub use response::{EndpointDispatch, EndpointOutcomeKind, LiveEndpointResponse};
pub use service::{
    EndpointFuture, EndpointKernel, LiveEndpointService, VerifiedEndpointRequest,
    VerifiedEndpointSnapshot,
};
