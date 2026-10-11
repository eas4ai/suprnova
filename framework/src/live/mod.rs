//! Server-driven interactive components for ordinary Suprnova applications.
//!
//! Live keeps routes and initial HTML server-rendered while allowing an island
//! to synchronize typed state and invoke registered Rust actions through the
//! shipped browser runtime.

pub mod action;
/// Exact reviewed browser artifacts, their catalog, and document bootstrap markup.
pub mod assets;
mod async_transport;
pub(crate) mod async_updates;
pub(crate) mod attestation;
mod config;
pub(crate) mod context;
mod document;
mod events;
pub(crate) mod ports;
mod registry;
mod replay;
mod response;
pub(crate) mod revocation;
mod routes;
mod runtime;
pub(crate) mod session_state;
mod ui_assets;
pub(crate) use runtime::LiveMountRegistration;
// `render_cache::RenderCache::install` derives its own `SnapshotKeyRing`
// from the same root key material Live uses (purpose separation keeps the
// two rings' MACs distinct; see `runtime::build_key_ring`'s doc and ruling
// R56). `mod runtime` above is private, so re-exporting the function here
// is what makes `crate::live::build_key_ring()` reachable from a sibling
// module (`render_cache`) rather than only from within `live` itself.
pub(crate) use runtime::build_key_ring;
mod streams;
mod tenant;
mod upload;
mod upload_host;
mod upload_policy;

/// Focused assertion helpers for application component tests.
pub mod testing;
/// Hidden console helper answering the CLI's Live tooling protocol.
pub mod tooling;
/// Bounded JSON-lines protocol between the CLI and the tooling helper.
pub mod tooling_protocol;

/// Hidden generated-code support. Application code must not use this module.
#[doc(hidden)]
pub mod __private;

pub use assets::{LiveBootstrap, LiveBootstrapOptions, LiveBootstrapStrategy};
pub use config::{
    LedgerDriver, LiveConfig, LiveConfigBuilder, LiveConfigError, LiveConfigErrorKind,
    LiveLimitExceeded, LiveMorphLimits, LiveServerLimits, LiveUploadLimits,
};
pub(crate) use document::NestedSegmentIdentity;
pub use document::{
    LiveDocument, LiveDocumentError, LiveDocumentErrorKind, LiveMount, LiveMountKind,
    LiveNestedSegment, MountedIsland, StitchFailurePolicy, StitchSlotDescriptor,
};
pub use registry::{
    ComponentContract, LiveRegistry, LiveRegistryBuilder, RegistryError, RegistryErrorKind,
};
pub use routes::LiveRouteGuard;
pub use runtime::{
    LiveRuntime, production_ledger_limits, verify_ledger_backend, verify_ledger_driver_for_test,
};
pub use streams::{LiveEventTarget, LiveStreamError, LiveStreamErrorKind, LiveStreams};
pub use suprnova_macros::{LiveComponent, live};
pub use tenant::{LiveTenantMiddleware, LiveTenantResolver, current_tenant};
pub use ui_assets::{LIVE_UI_ASSET_PATH_PREFIX, LIVE_UI_TEMPLATE_ROOT};

/// Server-rendered charts for the data-display component family.
pub mod charts {
    pub use suprnova_live::view::charts::{
        ChartError, ChartErrorKind, ChartKind, ChartSeries, render_chart,
    };
}

/// Versioned browser event and effect contracts declared by Live components.
pub mod metadata {
    pub use suprnova_live::metadata::{EffectPayloadMetadata, EventPayloadMetadata};
}

/// The record store contract behind Live's distributed instance ledger:
/// what the SQL and Redis record stores in
/// [`render_cache::providers`](crate::render_cache::providers) implement,
/// and the keys, records, outcomes and errors their methods take and
/// return.
pub mod ledger {
    pub use suprnova_live::ledger::{
        CasOutcome, InstanceRecordKey, InstanceRecordStore, LedgerError, LedgerErrorKind,
        PromotionRecordKey, StoredRecord,
    };
}

/// Stable redacted failure and browser-recovery contracts.
pub mod error {
    pub use suprnova_live::error::{
        ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode,
    };
}

/// Bounded localizable validation contracts used by Live components.
pub mod validation {
    pub use suprnova_live::validation::{
        BagPolicy, ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus,
    };
}

pub use action::{ActionOutcome, ActionResult, AuthorizedAction};
pub use error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};
pub use events::{AcceptedOutcomeKind, LiveOutcomeAccepted};
pub use metadata::{EffectPayloadMetadata, EventPayloadMetadata};
pub use suprnova_live::canonical::CanonicalValue;
/// The wall clock Live and the RenderCache read. Re-exported so an
/// application can implement it, or hold one as `Arc<dyn Clock>`, as
/// [`AdjustableTestClock`](crate::live::testing::AdjustableTestClock) and
/// [`RenderCacheConfig::with_clock`](crate::render_cache::RenderCacheConfig::with_clock)
/// do.
pub use suprnova_live::clock::Clock;
/// The failure a [`Clock`] reports, and its closed
/// reason.
pub use suprnova_live::clock::{ClockError, ClockErrorKind};
pub use suprnova_live::identity::UnixMillis;
/// The typed identities a
/// [`StitchSlotDescriptor`] records.
pub use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity,
};
pub use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
/// The server-declared document mount key a
/// [`StitchSlotDescriptor`] records.
pub use suprnova_live::mount::DocumentMountKey;
pub use suprnova_live::mount::MountFlags;
/// The request-lifetime cancellation flag the Live runtime attaches to a
/// prepared request, which
/// [`request_cancellation_for_test`](crate::live::testing::request_cancellation_for_test)
/// returns.
pub use suprnova_live::resource::CancellationFlag;
/// The including document's resolved path, which
/// [`LiveNestedSegment::into_segment`](crate::live::LiveNestedSegment::into_segment)
/// checks.
pub use suprnova_live::snapshot::MountedDocumentPath;
pub use suprnova_live::upload::{
    BoundedHeaders, ChunkDisposition, ChunkReceipt, DirectPartReference, DirectTransferInstruction,
    DirectUploadProvider, DurableUpload, DurableUploadId, FailedFinalize, FinalizeRequest,
    FinalizeToken, IntegrityEvidence, PrepareTransfer, PreparedFinalize, QuarantineBytes,
    ReadUpload, ReportDirectPart, ScanDisposition, ScanInput, TransferDisposition,
    TransferInstruction, TransferMethod, TransferPlan, TrustedProviderOrigin, TrustedProviderUrl,
    UploadApplicationValidator, UploadError, UploadErrorKind, UploadFinalizer, UploadFuture,
    UploadHandle, UploadPart, UploadProvider, UploadScanner, VerifyTransfer,
};
pub use upload_host::LiveUploadHost;
pub use upload_policy::{
    UploadPolicy, UploadPolicyBuilder, UploadPolicyError, UploadReplacement, UploadScan,
    UploadScanFailure, UploadType,
};
pub use validation::{ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus};
