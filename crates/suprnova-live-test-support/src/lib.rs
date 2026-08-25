//! Development-only conformance support for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

mod assertions;
mod context;
mod file_quarantine_store;
mod harness;
mod host;
mod trace;
mod upload;

pub use assertions::HarnessAssertions;
pub use context::SyntheticLiveRequestContextBuilder;
pub use file_quarantine_store::{FileStoreFault, TokioFileQuarantineStore};
pub use harness::{
    ComponentHarness, ComponentHarnessConfig, HarnessError, HarnessErrorKind, HarnessMount,
    HarnessRequestIdentity,
};
pub use host::{
    ControlledAuthorization, ControlledClock, ControlledInstanceIds, ControlledSession,
    ControlledTransactions, ControlledValidation, HarnessServices, TransactionFault,
};
pub use trace::{HarnessTrace, HarnessTraceEvent};
pub use upload::{ControlledUploadAuthorization, MemoryUploadLedger};
