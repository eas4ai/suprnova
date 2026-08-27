//! Development-only conformance support for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

mod assertions;
mod async_reference_host;
mod context;
mod direct_provider;
mod file_quarantine_store;
mod harness;
mod host;
mod reference_host;
mod trace;
mod upload;

pub use assertions::HarnessAssertions;
pub use async_reference_host::{
    ASYNC_REFERENCE_ORIGIN, ASYNC_REFERENCE_PRINCIPAL, ASYNC_REFERENCE_SCOPE,
    ASYNC_REFERENCE_SESSION, AsyncReferenceAuthority, AsyncReferenceAuthorizationRequest,
    AsyncReferenceFault, AsyncReferenceMembershipRequest, AsyncReferencePollRequest,
    AsyncReferencePosition, AsyncReferenceScenario,
};
pub use context::SyntheticLiveRequestContextBuilder;
pub use direct_provider::DirectProviderConformanceAdapter;
pub use file_quarantine_store::{FileStoreFault, TokioFileQuarantineStore};
pub use harness::{
    ComponentHarness, ComponentHarnessConfig, HarnessError, HarnessErrorKind, HarnessMount,
    HarnessRequestIdentity,
};
pub use host::{
    ControlledAuthorization, ControlledClock, ControlledInstanceIds, ControlledSession,
    ControlledTransactions, ControlledValidation, HarnessServices, TransactionFault,
};
pub use reference_host::{
    CREATE_UPLOAD, EXAMPLE_REACQUIRE, POLL, REFERENCE_AUTHORIZATION, ReferenceFaultSchedule,
    ReferenceHost, ReferenceHostConfig, ReferenceHostInspection, ReferenceHostInspectionHandle,
    SSE, TRANSPORT_CREATE, TRANSPORT_MEMBERSHIP, UPLOAD_CANCEL, UPLOAD_CHUNK, UPLOAD_COMPLETE,
    UPLOAD_STATUS, WEBSOCKET,
};
pub use trace::{HarnessTrace, HarnessTraceEvent};
pub use upload::{ControlledUploadAuthorization, MemoryCleanupObservation, MemoryUploadLedger};
