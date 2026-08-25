//! Opaque temporary upload identities and secret transfer capabilities.

mod identity;
mod protocol;
mod state;

pub use identity::{
    IssuedTransferGrant, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, UploadError, UploadErrorKind, UploadHandle, VerifiedTransferGrant,
};
pub use protocol::{
    CancelUpload, CompleteUpload, CreateUpload, PutChunk, ReacquireUpload,
    SUPPORTED_UPLOAD_PROTOCOL_VERSIONS, StatusUpload, UploadChecksum, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadRevision,
};
pub use state::{
    AcceptedChunk, TransitionDisposition, TransitionOutcome, UploadState, UploadStateMachine,
    UploadTransition, UploadTransitionRequest,
};
