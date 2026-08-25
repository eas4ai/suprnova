//! Request-scoped upload admission over host-owned conditional authority.

use std::fmt;
use std::sync::Arc;

use crate::host::TrustedLiveRequestContext;
use crate::identity::{ComponentName, ModelField, UnixMillis};
use crate::limits::UploadLimits;
use crate::resource::{CancellationFlag, PermitPool, ResourceBounds, ResourceOwner, Retirement};

use super::{
    ConditionalTransition, ConditionalUploadCreate, TransferGrant, TransferGrantCodec,
    TransferGrantRequest, TransferGrantScope, TransitionOutcome, UploadCreateCommand, UploadError,
    UploadErrorKind, UploadFuture, UploadHandle, UploadIdempotencyKey, UploadLedger,
    UploadLedgerCreateOutcome, UploadRecord, UploadRevision, UploadState, UploadTransition,
    UploadTransitionRequest,
};

const UPLOAD_PROTOCOL_V1: u16 = 1;

/// Closed upload-control identity supplied to current authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadControlKind {
    /// Creates one temporary upload authority record.
    Create,
    /// Reads current state without mutation.
    Status,
    /// Admits a created upload to the resource queue.
    Queue,
    /// Starts transfer work.
    BeginTransfer,
    /// Records one accepted chunk.
    PutChunk,
    /// Ends byte transfer and starts verification.
    Complete,
    /// Accepts authoritative verification.
    Accept,
    /// Starts durable finalization.
    BeginFinalize,
    /// Commits durable finalization.
    CommitFinalize,
    /// Cancels pending work.
    Cancel,
    /// Rejects authoritative verification.
    Reject,
    /// Expires temporary authority.
    Expire,
    /// Closes work after a provider or host failure.
    Fail,
}

impl UploadControlKind {
    const fn from_transition(transition: &UploadTransition) -> Self {
        match transition {
            UploadTransition::Queue => Self::Queue,
            UploadTransition::BeginTransfer => Self::BeginTransfer,
            UploadTransition::PutChunk(_) => Self::PutChunk,
            UploadTransition::Complete => Self::Complete,
            UploadTransition::Accept => Self::Accept,
            UploadTransition::BeginFinalize => Self::BeginFinalize,
            UploadTransition::CommitFinalize => Self::CommitFinalize,
            UploadTransition::Cancel => Self::Cancel,
            UploadTransition::Reject => Self::Reject,
            UploadTransition::Expire => Self::Expire,
            UploadTransition::Fail => Self::Fail,
        }
    }
}

/// Closed current-policy result returned by the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadAuthorizationDecision {
    /// Current request authority permits this upload control.
    Allow,
    /// Current request authority denies this upload control.
    Deny,
}

/// Safe registered identities supplied to current upload authorization.
#[derive(Clone, Copy)]
pub struct UploadAuthorizationRequest<'a> {
    component: &'a ComponentName,
    field: &'a ModelField,
    handle: &'a UploadHandle,
    control: UploadControlKind,
}

impl<'a> UploadAuthorizationRequest<'a> {
    const fn new(
        component: &'a ComponentName,
        field: &'a ModelField,
        handle: &'a UploadHandle,
        control: UploadControlKind,
    ) -> Self {
        Self {
            component,
            field,
            handle,
            control,
        }
    }

    /// Returns the registry-verified component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.component
    }

    /// Returns the declared upload model field.
    #[must_use]
    pub const fn field(&self) -> &ModelField {
        self.field
    }

    /// Returns the non-authoritative temporary upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.handle
    }

    /// Returns the requested closed control identity.
    #[must_use]
    pub const fn control(&self) -> UploadControlKind {
        self.control
    }
}

impl fmt::Debug for UploadAuthorizationRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadAuthorizationRequest")
            .field("component", &self.component.as_str())
            .field("field", &self.field.as_str())
            .field("handle", &"<redacted>")
            .field("control", &self.control)
            .finish()
    }
}

/// Host-owned current principal and resource policy for upload controls.
pub trait UploadAuthorizationPort: Send + Sync {
    /// Rechecks current authority for exactly one upload control boundary.
    fn authorize<'a>(
        &'a self,
        request: UploadAuthorizationRequest<'a>,
    ) -> UploadFuture<'a, Result<UploadAuthorizationDecision, UploadError>>;
}

/// Trusted internal request to create one temporary upload.
#[derive(Clone, Eq, PartialEq)]
pub struct UploadCreationRequest {
    handle: UploadHandle,
    field: ModelField,
    idempotency_key: UploadIdempotencyKey,
    expires_at: UnixMillis,
}

impl UploadCreationRequest {
    /// Groups a server-generated handle with its declared field, retry identity, and expiry.
    #[must_use]
    pub const fn new(
        handle: UploadHandle,
        field: ModelField,
        idempotency_key: UploadIdempotencyKey,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            handle,
            field,
            idempotency_key,
            expires_at,
        }
    }

    /// Returns the server-generated non-authoritative handle.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the declared upload model field.
    #[must_use]
    pub const fn field(&self) -> &ModelField {
        &self.field
    }

    /// Returns the creation retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the exclusive temporary-authority expiry instant.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }
}

impl fmt::Debug for UploadCreationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCreationRequest:redacted>")
    }
}

/// Secret-bearing admitted transition request kept out of diagnostics.
#[derive(Clone)]
pub struct UploadTransitionAdmission {
    grant: TransferGrant,
    field: ModelField,
    transition: UploadTransitionRequest,
}

impl UploadTransitionAdmission {
    /// Groups a bearer grant with its declared field and conditional transition.
    #[must_use]
    pub const fn new(
        grant: TransferGrant,
        field: ModelField,
        transition: UploadTransitionRequest,
    ) -> Self {
        Self {
            grant,
            field,
            transition,
        }
    }
}

impl fmt::Debug for UploadTransitionAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadTransitionAdmission:redacted>")
    }
}

/// Complete successful creation result with a separately secret transfer grant.
pub struct UploadCreateOutcome {
    disposition: ConditionalUploadCreate,
    record: UploadRecord,
    grant: TransferGrant,
}

impl UploadCreateOutcome {
    /// Returns whether the record was created or exactly replayed.
    #[must_use]
    pub const fn disposition(&self) -> ConditionalUploadCreate {
        self.disposition
    }

    /// Returns the authoritative non-secret record.
    #[must_use]
    pub const fn record(&self) -> &UploadRecord {
        &self.record
    }

    /// Returns the separate short-lived bearer capability.
    #[must_use]
    pub const fn grant(&self) -> &TransferGrant {
        &self.grant
    }
}

impl fmt::Debug for UploadCreateOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCreateOutcome:redacted>")
    }
}

/// Conditional upload service over one host ledger and current-policy capability.
pub struct UploadService {
    ledger: Arc<dyn UploadLedger>,
    grants: TransferGrantCodec,
    limits: UploadLimits,
    resources: ResourceOwner<UploadHandle>,
    transfer_permits: PermitPool,
}

impl UploadService {
    /// Creates one bounded service using the shared resource foundation.
    pub fn new(
        ledger: Arc<dyn UploadLedger>,
        grants: TransferGrantCodec,
        limits: UploadLimits,
    ) -> Result<Self, UploadError> {
        let bounds = ResourceBounds::new(
            limits.max_concurrent_transfers(),
            limits.max_in_flight_bytes(),
        )
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        let transfer_permits = PermitPool::new(limits.max_concurrent_transfers())
            .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
        Ok(Self {
            ledger,
            grants,
            limits,
            resources: ResourceOwner::new(bounds),
            transfer_permits,
        })
    }

    /// Atomically creates upload authority and issues its separate bearer grant.
    pub async fn create(
        &self,
        context: &TrustedLiveRequestContext,
        request: UploadCreationRequest,
        now: UnixMillis,
    ) -> Result<UploadCreateOutcome, UploadError> {
        Self::require_current(context, now)?;
        self.authorize_current(
            context,
            &request.handle,
            &request.field,
            UploadControlKind::Create,
            now,
        )
        .await?;
        if request.expires_at <= now
            || request.expires_at.get().saturating_sub(now.get()) > self.limits.max_age_ms()
        {
            return Err(UploadError::new(UploadErrorKind::UploadExpired));
        }
        self.require_active()?;

        let authority = self.authority(context, request.handle, request.field);
        let issued = self.grants.issue(
            TransferGrantRequest::new(authority.clone(), request.expires_at),
            now,
        )?;
        let record = UploadRecord::new(
            authority,
            UploadState::Created,
            UploadRevision::initial(),
            now,
            request.expires_at,
        )?;
        let outcome = self
            .ledger
            .create(UploadCreateCommand::new(
                record,
                request.idempotency_key,
                now,
                self.limits,
            ))
            .await?;
        Ok(Self::creation_outcome(outcome, issued))
    }

    /// Verifies all request authority and atomically applies one lifecycle transition.
    pub async fn transition(
        &self,
        context: &TrustedLiveRequestContext,
        admission: UploadTransitionAdmission,
        now: UnixMillis,
    ) -> Result<TransitionOutcome, UploadError> {
        Self::require_current(context, now)?;
        let handle = admission.transition.handle().clone();
        let authority = self.authority(context, handle, admission.field.clone());
        self.grants.verify(&admission.grant, &authority, now)?;
        self.authorize_current(
            context,
            authority.handle(),
            authority.field(),
            UploadControlKind::from_transition(admission.transition.transition()),
            now,
        )
        .await?;
        self.validate_transition_limits(&admission.transition)?;
        self.require_active()?;
        self.ledger
            .transition(ConditionalTransition::new(
                authority,
                admission.transition,
                now,
            ))
            .await
    }

    /// Reauthorizes and loads current status without mutating the upload record.
    pub async fn status(
        &self,
        context: &TrustedLiveRequestContext,
        grant: TransferGrant,
        field: ModelField,
        handle: UploadHandle,
        now: UnixMillis,
    ) -> Result<UploadRecord, UploadError> {
        Self::require_current(context, now)?;
        let authority = self.authority(context, handle, field);
        self.grants.verify(&grant, &authority, now)?;
        self.authorize_current(
            context,
            authority.handle(),
            authority.field(),
            UploadControlKind::Status,
            now,
        )
        .await?;
        self.require_active()?;
        let record = self
            .ledger
            .load(authority.handle())
            .await?
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if record.authority() != &authority {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }
        if record.expires_at() <= now {
            return Err(UploadError::new(UploadErrorKind::UploadExpired));
        }
        Ok(record)
    }

    pub(crate) async fn trusted_status(
        &self,
        context: &TrustedLiveRequestContext,
        field: ModelField,
        handle: UploadHandle,
        control: UploadControlKind,
        now: UnixMillis,
    ) -> Result<UploadRecord, UploadError> {
        Self::require_current(context, now)?;
        let authority = self.authority(context, handle, field);
        self.authorize_current(context, authority.handle(), authority.field(), control, now)
            .await?;
        self.require_active()?;
        let record = self
            .ledger
            .load(authority.handle())
            .await?
            .ok_or_else(|| UploadError::new(UploadErrorKind::UploadConflict))?;
        if record.authority() != &authority {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }
        if record.expires_at() <= now {
            return Err(UploadError::new(UploadErrorKind::UploadExpired));
        }
        Ok(record)
    }

    pub(crate) async fn trusted_transition(
        &self,
        context: &TrustedLiveRequestContext,
        field: ModelField,
        transition: UploadTransitionRequest,
        now: UnixMillis,
    ) -> Result<TransitionOutcome, UploadError> {
        Self::require_current(context, now)?;
        let authority = self.authority(context, transition.handle().clone(), field);
        self.authorize_current(
            context,
            authority.handle(),
            authority.field(),
            UploadControlKind::from_transition(transition.transition()),
            now,
        )
        .await?;
        self.validate_transition_limits(&transition)?;
        self.require_active()?;
        self.ledger
            .transition(ConditionalTransition::new(authority, transition, now))
            .await
    }

    /// Returns a cloneable observer canceled when this service retires.
    #[must_use]
    pub fn cancellation(&self) -> CancellationFlag {
        self.resources.cancellation()
    }

    /// Returns the shared bounded transfer-permit pool.
    #[must_use]
    pub const fn transfer_permits(&self) -> &PermitPool {
        &self.transfer_permits
    }

    /// Retires this service and cancels all future owned work admission.
    pub fn retire(&self) -> Retirement {
        self.resources.retire()
    }

    fn authority(
        &self,
        context: &TrustedLiveRequestContext,
        handle: UploadHandle,
        field: ModelField,
    ) -> TransferGrantScope {
        TransferGrantScope::new(
            handle,
            context.mount().component().clone(),
            field,
            context.host_scope_facts().clone(),
            UPLOAD_PROTOCOL_V1,
        )
    }

    async fn authorize_current(
        &self,
        context: &TrustedLiveRequestContext,
        handle: &UploadHandle,
        field: &ModelField,
        control: UploadControlKind,
        _now: UnixMillis,
    ) -> Result<(), UploadError> {
        let authorization = context
            .capabilities()
            .upload_authorization()
            .ok_or_else(|| UploadError::new(UploadErrorKind::AuthorizationUnavailable))?;
        let request =
            UploadAuthorizationRequest::new(context.mount().component(), field, handle, control);
        match authorization.authorize(request).await? {
            UploadAuthorizationDecision::Allow => Ok(()),
            UploadAuthorizationDecision::Deny => {
                Err(UploadError::new(UploadErrorKind::AuthorizationDenied))
            }
        }
    }

    fn validate_transition_limits(
        &self,
        request: &UploadTransitionRequest,
    ) -> Result<(), UploadError> {
        if let UploadTransition::PutChunk(chunk) = request.transition()
            && (chunk.size() > self.limits.max_chunk_bytes() as u64
                || chunk.size() > self.limits.max_file_bytes())
        {
            return Err(UploadError::new(UploadErrorKind::InputTooLarge));
        }
        Ok(())
    }

    fn require_active(&self) -> Result<(), UploadError> {
        if self.resources.cancellation().is_canceled() {
            Err(UploadError::new(UploadErrorKind::ServiceRetired))
        } else {
            Ok(())
        }
    }

    fn require_current(
        context: &TrustedLiveRequestContext,
        now: UnixMillis,
    ) -> Result<(), UploadError> {
        if context.is_current(now) {
            Ok(())
        } else {
            Err(UploadError::new(UploadErrorKind::RequestAuthorityExpired))
        }
    }

    fn creation_outcome(
        outcome: UploadLedgerCreateOutcome,
        issued: super::IssuedTransferGrant,
    ) -> UploadCreateOutcome {
        UploadCreateOutcome {
            disposition: outcome.disposition(),
            record: outcome.record().clone(),
            grant: issued.grant().clone(),
        }
    }
}

impl fmt::Debug for UploadService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadService")
            .field("limits", &self.limits)
            .field("resources", &self.resources)
            .field("transfer_permits", &self.transfer_permits)
            .finish_non_exhaustive()
    }
}
