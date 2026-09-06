//! Tier-independent Live instance revision-authority contract and Tier 0 provider.

mod contract;
mod distributed;
mod memory;
mod record;
mod state;

pub use contract::{
    AcceptedOutcome, AcceptedOutcomeKind, AcceptedOutcomeMetadata, ClaimGrant, ClaimOutcome,
    ClaimRequest, ClaimToken, InstanceAuthority, LedgerError, LedgerErrorKind, LedgerInspection,
    LedgerLimits, LedgerPhase, LiveInstanceLedger, MountInstanceRecord, PromotionOutcome,
    PromotionRecord, RefreshReason,
};
pub use distributed::{
    CasOutcome, CleanupOp, InstanceRecordKey, InstanceRecordStore, MemoryRecordStore,
    PromotionRecordKey, StoredRecord,
};
pub use memory::MemoryInstanceLedger;
pub use record::{MAX_RECORD_BYTES, RECORD_VERSION};
