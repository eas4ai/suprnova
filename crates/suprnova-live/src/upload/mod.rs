//! Opaque temporary upload identities and secret transfer capabilities.

mod cleanup;
mod direct_provider;
mod finalize;
mod identity;
mod ledger;
mod policy;
mod protocol;
mod provider;
mod quarantine;
mod service;
mod state;
mod telemetry;
mod validation;

pub use cleanup::{
    BoundedBackoff, CleanupBatchRequest, CleanupClaim, CleanupCompletion, CleanupCompletionKind,
    CleanupDisposition, CleanupLeaseId, CleanupLedgerDisposition, CleanupPolicy, CleanupRunOutcome,
    UploadCleanupLedger, UploadCleanupService,
};
pub use direct_provider::{
    BoundedHeaders, DirectPartReference, DirectTransferInstruction, ReportDirectPart,
    TransferInstruction, TransferMethod, TrustedProviderOrigin, TrustedProviderUrl, UploadPart,
};
pub use finalize::{
    DurableUpload, DurableUploadId, FailedFinalize, FinalizeDisposition, FinalizeFailureStage,
    FinalizeRequest, FinalizeToken, FinalizeUploadOutcome, FinalizeUploadRequest, PreparedFinalize,
    UploadFinalizationService, UploadFinalizer,
};
pub use identity::{
    IssuedTransferGrant, TransferGrant, TransferGrantCodec, TransferGrantRequest,
    TransferGrantScope, UploadError, UploadErrorKind, UploadHandle, VerifiedTransferGrant,
};
pub use ledger::{
    ConditionalTransition, ConditionalUploadCreate, UploadCreateCommand, UploadFuture,
    UploadLedger, UploadLedgerCreateOutcome, UploadRecord,
};
pub use policy::{
    AcceptedUploadType, AuthoritativeUploadType, ScanFailurePolicy, UploadDimensionLimits,
    UploadFieldPolicy, UploadMediaType, UploadReplacementPolicy, UploadScanPolicy,
};
pub use protocol::{
    CancelUpload, CompleteUpload, CreateUpload, PutChunk, ReacquireUpload,
    SUPPORTED_UPLOAD_PROTOCOL_VERSIONS, StatusUpload, UploadChecksum, UploadIdempotencyKey,
    UploadOperation, UploadProtocolCodec, UploadRevision,
};
pub use provider::{
    CheckpointChunk, ChunkBody, ChunkDisposition, ChunkReceipt, DirectUploadProvider,
    IntegrityEvidence, PrepareTransfer, ProviderRetirementError, ProviderRetirementStatus,
    ProviderTransferAccounting, QuarantinedFileProvider, ReadUpload, ReverseProxyUploadProvider,
    TransferCheckpoint, TransferDisposition, TransferPlan, UploadProvider, VerifyTransfer,
    WriteChunk,
};
pub use quarantine::{
    QuarantineBytes, QuarantineCompletion, QuarantineObject, QuarantineOperation, QuarantineStore,
    RemoveDisposition,
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
pub use telemetry::{
    CleanupMetricSink, CleanupMetrics, CleanupOutcome, RetryBucket, UploadAgeBucket,
    UploadVolumeBucket,
};
pub use validation::{
    ApplicationValidationDecision, ApplicationValidationInput, ClientUploadMetadata,
    DetectedUploadType, MediaDimensions, MediaHeaderProbe, ScanDisposition, ScanInput, ScanReason,
    UploadApplicationValidator, UploadContent, UploadInspection, UploadRejectionReason,
    UploadScanner, UploadValidationDisposition, UploadValidationOutcome, UploadValidationRequest,
    UploadValidationService, UploadValidationStore, ValidatedUpload, ValidationStoreDisposition,
};
