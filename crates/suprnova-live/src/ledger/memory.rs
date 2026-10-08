//! Complete single-process Tier 0 instance-ledger provider.
//!
//! There is no Tier 0 state machine. This provider is
//! [`DistributedInstanceLedger`] over [`MemoryRecordStore`], so the embedded
//! deployment runs exactly the transitions a database-coordinated or
//! externally accelerated one runs, and the conformance suite that proves one
//! proves the other.
//!
//! Two things stay in this shape because callers depend on them. Inspection
//! is synchronous, and a dropped claim is retired before the next statement
//! rather than at the next asynchronous operation: an in-process store has no
//! remote input or output to block on, so deferring either would cost
//! correctness that the embedded tier has always given.

use std::sync::Arc;

use async_trait::async_trait;

use super::distributed::{CleanupOp, DistributedInstanceLedger, MemoryRecordStore};
use super::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerInspection, LedgerLimits, LiveInstanceLedger, MountInstanceRecord, PromotionOutcome,
    PromotionRecord,
};
use crate::clock::Clock;
use crate::identity::{InstanceId, Revision, ScopeFingerprint};

/// Complete zero-daemon instance revision authority for one application process.
#[derive(Clone)]
pub struct MemoryInstanceLedger {
    kernel: DistributedInstanceLedger<MemoryRecordStore>,
}

impl MemoryInstanceLedger {
    /// Creates an empty bounded provider using the injected wall clock.
    ///
    /// The same clock measures instance lifetimes and backs the in-process
    /// store's expiry, which is what makes a single process one consistent
    /// timeline.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, limits: LedgerLimits) -> Self {
        let store = Arc::new(MemoryRecordStore::new(Arc::clone(&clock)));
        Self {
            kernel: DistributedInstanceLedger::new(store, clock, limits),
        }
    }

    /// Returns metadata-only provider inspection for tests and trusted diagnostics.
    pub fn inspect(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<LedgerInspection>, LedgerError> {
        self.kernel.inspect_in_process(scope, instance_id)
    }
}

#[async_trait]
impl LiveInstanceLedger for MemoryInstanceLedger {
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        self.kernel.mount_instance(record).await
    }

    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        self.kernel.promote(request).await
    }

    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        self.kernel.claim(request).await
    }

    async fn current_accepted_revision(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<Revision>, LedgerError> {
        self.kernel
            .current_accepted_revision(scope, instance_id)
            .await
    }

    async fn commit(
        &self,
        claim: &ClaimToken,
        outcome: AcceptedOutcome,
    ) -> Result<(), LedgerError> {
        self.kernel.commit(claim, outcome).await
    }

    async fn abandon(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        self.kernel.abandon(claim).await
    }

    fn abandon_on_drop(&self, claim: ClaimToken) {
        self.kernel.retire_in_process(CleanupOp::Abandon(claim));
    }

    fn fence_on_drop(&self, claim: ClaimToken) {
        self.kernel.retire_in_process(CleanupOp::Fence(claim));
    }
}
