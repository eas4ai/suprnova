//! Opaque temporary upload identities and secret transfer capabilities.

mod identity;
mod ledger;
mod protocol;
mod service;
mod state;

pub use identity::{
    IssuedTransferGrant, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, UploadError, UploadErrorKind, UploadHandle, VerifiedTransferGrant,
};
pub use ledger::{
    ConditionalTransition, ConditionalUploadCreate, UploadCreateCommand, UploadFuture,
    UploadLedger, UploadLedgerCreateOutcome, UploadRecord,
};
pub use protocol::{
    CancelUpload, CompleteUpload, CreateUpload, PutChunk, ReacquireUpload,
    SUPPORTED_UPLOAD_PROTOCOL_VERSIONS, StatusUpload, UploadChecksum, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadRevision,
};
pub use service::{
    UploadAuthorizationDecision, UploadAuthorizationPort, UploadAuthorizationRequest,
    UploadControlKind, UploadCreateOutcome, UploadCreationRequest, UploadService,
    UploadTransitionAdmission,
};
pub use state::{
    AcceptedChunk, TransitionDisposition, TransitionOutcome, UploadState, UploadStateMachine,
    UploadTransition, UploadTransitionRequest,
};
