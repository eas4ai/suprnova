//! Server-driven interactive components for ordinary Suprnova applications.
//!
//! Live keeps routes and initial HTML server-rendered while allowing an island
//! to synchronize typed state and invoke registered Rust actions through the
//! shipped browser runtime.

pub mod action;
#[cfg(feature = "agents")]
mod agent;
#[cfg(feature = "agents")]
pub(crate) use agent::RESULT_MEDIA_TYPE as LIVE_TOOL_RESULT_MEDIA_TYPE;
/// Exact reviewed browser artifacts, their catalog, and document bootstrap markup.
pub mod assets;
mod async_transport;
pub(crate) mod async_updates;
pub(crate) mod attestation;
/// Scoped interactive controls registered explicitly by the application.
pub mod components;
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
pub(crate) use routes::{LIVE_AGENT_PATH, LIVE_UPDATE_PATH};
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
    /// The canonical metadata of a component contract: the component's
    /// own, its fields, actions, events and effects, the contract versions
    /// it binds and an event payload contract's identity, with the failure,
    /// and its closed reason, a construction reports. A
    /// [`ComponentDescriptor`](crate::live::ComponentDescriptor) and
    /// [`ComponentInstance`](crate::live::ComponentInstance) hand them to an
    /// application.
    pub use suprnova_live::metadata::{
        ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EventMetadata,
        FieldMetadata, MetadataError, MetadataErrorKind, PayloadContractIdentity,
    };
    /// What `#[tool]` declares on an action, the grammar a tool
    /// component's island key follows, and the agent-visible fields a tool
    /// result copies from a component's state (AGENT-012, AGENT-013).
    pub use suprnova_live::metadata::{
        ActionToolMetadata, MAX_TOOL_BASE_BYTES, MAX_TOOL_INSTANCE_BYTES, agent_fields,
        is_tool_instance_key,
    };
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
    /// The failure [`ErrorBag::from_issues`] and
    /// [`ValidationMessageId::parse`] report: a message identity that is not
    /// valid, or more issues than a bag holds.
    pub use suprnova_live::validation::ValidationBagError;
    /// The validation work an action selects, which an action's
    /// [`ActionMetadata`](crate::live::metadata::ActionMetadata) declares.
    pub use suprnova_live::validation::ValidationSelection;
    pub use suprnova_live::validation::{
        BagPolicy, ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus, ValidationText,
    };
}

pub use action::{ActionOutcome, ActionResult, AuthorizedAction};
pub use error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};
pub use events::{AcceptedOutcomeKind, LiveOutcomeAccepted};
pub use metadata::{EffectPayloadMetadata, EventPayloadMetadata};
/// The host ports a subscription goes through, with the requests they read
/// and the decisions and outcomes they return: authorization, stream
/// continuity, transport credentials and the trusted registry lookup. A
/// [`HostCapabilities`] takes the ports.
pub use suprnova_live::async_updates::{
    AuthoritativeStreamPosition, CurrentSubscriptionRegistration,
    SubscriptionAuthorizationDecision, SubscriptionAuthorizationOperation,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionBinding, SubscriptionContinuityPort, SubscriptionCredentialPort,
    SubscriptionCredentialRequest, SubscriptionCredentialRotationOutcome,
    SubscriptionCredentialRotationRequest, SubscriptionCredentialScope, SubscriptionFuture,
    SubscriptionRegistryPort, SubscriptionRegistryRequest, TrustedMountParameters,
};
/// What the subscription ports' requests and outcomes carry: the stream
/// position a continuity authority mints, the event contracts of a stream,
/// the authorization memo and the transport credential a credential port
/// issues, and the failure, with its closed reason, a subscription reports.
pub use suprnova_live::async_updates::{
    AuthorizationMemo, BoundedEventContracts, StreamEpoch, StreamPosition, StreamSequence,
    SubscriptionError, SubscriptionErrorKind, SubscriptionEventContract, TransportCredential,
};
/// The asynchronous stream contracts a component declares: its
/// subscriptions with their topics, transports and reconnect policy, and
/// its events with their targets, order, source, cycle policy and browser
/// payload schema. A [`ComponentMetadata`](crate::live::metadata::ComponentMetadata)
/// and an [`EventPayloadMetadata`] hand them to an application.
pub use suprnova_live::async_updates::{
    BoundedEventNames, BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, ReconnectPolicy, StreamName, SubscriptionMetadata,
    SubscriptionMode, SubscriptionModes, TopicName,
};
pub use suprnova_live::canonical::CanonicalValue;
/// The finite number a [`CanonicalValue`] holds, and the failure, with its
/// closed reason, a canonical value or number reports.
pub use suprnova_live::canonical::{CanonicalError, CanonicalErrorKind, CanonicalNumber};
/// The verified parameters a child component receives when its parent
/// changes them, which a [`ComponentInstance`] takes.
pub use suprnova_live::child::{EligibleChildParametersV2, VerifiedChildParametersV1};
/// The wall clock Live and the RenderCache read. Re-exported so an
/// application can implement it, or hold one as `Arc<dyn Clock>`, as
/// [`AdjustableTestClock`](crate::live::testing::AdjustableTestClock) and
#[cfg_attr(
    feature = "testing",
    doc = "[`RenderCacheConfig::with_clock`](crate::render_cache::RenderCacheConfig::with_clock)"
)]
#[cfg_attr(
    not(feature = "testing"),
    doc = "`RenderCacheConfig::with_clock` (with the `testing` feature)"
)]
/// do.
pub use suprnova_live::clock::Clock;
/// The failure a [`Clock`] reports, and its closed
/// reason.
pub use suprnova_live::clock::{ClockError, ClockErrorKind};
/// Child composition: the handle a surviving child keeps between parent
/// renders and its key, the registered parameter schema and its fields,
/// their digests, the pending parameters a parent hands a child, and the
/// failure, with its closed reason, a composition reports. A
/// [`ChildMount`](crate::view::ChildMount) and a [`ComponentDescriptor`]
/// hand them to an application.
pub use suprnova_live::component::composition::{
    ChildHandle, ChildKey, ChildParameterField, ChildParameterSchema, CompositionError,
    CompositionErrorKind, ParameterSchemaDigest, ParameterValueDigest, PendingChildParameters,
};
/// The generated hooks of a component and the request-owned instance they
/// create: the factory and the instance contract, their mount, hydration
/// and render contexts, the boxed future they return, and the failure, with
/// its closed category, they report. A [`ComponentDescriptor`] holds the
/// hooks, and a [`MountFailure`] carries the lifecycle failure kind.
pub use suprnova_live::component::{
    ComponentError, ComponentErrorKind, ComponentFactory, ComponentHooks, ComponentInstance,
    HydrationContext, LifecycleErrorKind, LiveFuture, MountContext, RenderContext,
};
/// The trusted host context a Live request runs in: the request context a
/// [`RenderContext`] carries, the host capabilities bound to its scope, the
/// fingerprints of that scope, the authenticity checks the request passed,
/// the registry-verified mount it renders and the browser tools its island
/// roots list for the viewer.
pub use suprnova_live::host::{
    AgentToolFilter, CheckDisposition, CheckKind, HostCapabilities, HostScopeFacts, PolicyReason,
    PrincipalFingerprint, RequiredChecks, SessionFingerprint, TenantFingerprint,
    TrustedLiveRequestContext, VerifiedMountCatalogMatch,
};
pub use suprnova_live::identity::UnixMillis;
/// The typed identities the Live engine items of this module carry: an
/// action and a browser operation, an instance and its revision, a model
/// field, the retry, correlation and nonce inputs of an update request, a
/// snapshot signing key, a scope digest, and the failure, with its closed
/// reason, a malformed one reports. [`AuthorizedAction`],
/// [`FlashIntent`](crate::live::action::FlashIntent), the record keys of
/// [`ledger`] and the constructors of [`BuildId`] and
/// its siblings hand them to an application.
pub use suprnova_live::identity::{
    ActionName, BrowserNonce, BrowserOperationName, CorrelationId, IdempotencyKey, IdentityError,
    IdentityErrorKind, InstanceId, KeyId, ModelField, Revision, ScopeFingerprint,
};
/// The typed identities a
/// [`StitchSlotDescriptor`] records.
pub use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity,
};
/// The Live engine's input bounds and size report: [`InputLimits`] is what
/// a [`FlashIntent`](crate::live::action::FlashIntent) or a
/// [`RouteIntent`](crate::live::action::RouteIntent) is checked against,
/// [`LimitConfigurationError`] is what [`UploadLimits::new`] refuses a
/// limit with, and [`SizeBreach`] is how far a rendered view went over its
/// limit, as a [`ViewError`](crate::view::ViewError) and a
/// [`MountFailure`] report it.
pub use suprnova_live::limits::{InputLimits, LimitConfigurationError, SizeBreach};
pub use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
/// The server-declared document mount key a
/// [`StitchSlotDescriptor`] records.
pub use suprnova_live::mount::DocumentMountKey;
pub use suprnova_live::mount::MountFlags;
/// The failure a refused mount reports, its coarse kind and its closed
/// cause, which [`DocumentMountKey::parse`] and [`MountFlags::new`] return.
pub use suprnova_live::mount::{MountError, MountErrorKind, MountFailure};
/// The seed-promotion authority a [`TrustedLiveRequestContext`] projects.
pub use suprnova_live::promotion::TrustedPromotionContext;
/// The browser's presentation context a [`RenderContext`] carries, the
/// parsed update request of either protocol version it is read from, with
/// its operations, snapshot input and child-parameter admission, and the
/// failure, with its closed reason, the protocol reports.
pub use suprnova_live::protocol::{
    BrowserRenderContext, ChildParameterAdmissionCarrier, Operation, OperationV2, ProtocolError,
    ProtocolErrorKind, SnapshotInput, UpdateRequest, UpdateRequestV2, VersionedUpdateRequest,
};
/// The closed reason the Live engine could not generate a server identity,
/// which a [`MountFailure`] carries.
pub use suprnova_live::random::RandomErrorKind;
/// The runtime descriptor generated for one component contract, which
/// [`ActionResult::new`] and a
/// [`RegisteredEmission`](crate::live::action::RegisteredEmission)'s
/// constructors check an emission against.
pub use suprnova_live::registry::ComponentDescriptor;
/// The closed reason the Live engine's own component registry refused a
/// registration or a lookup, which a [`MountFailure`] carries. It is named
/// apart from this module's [`RegistryErrorKind`], the reason a
/// [`LiveRegistry`] refuses a registration with.
pub use suprnova_live::registry::RegistryErrorKind as EngineRegistryErrorKind;
/// The request-lifetime cancellation flag the Live runtime attaches to a
/// prepared request, which
/// [`request_cancellation_for_test`](crate::live::testing::request_cancellation_for_test)
/// returns.
pub use suprnova_live::resource::CancellationFlag;
/// The including document's resolved path, which
/// [`LiveNestedSegment::into_segment`](crate::live::LiveNestedSegment::into_segment)
/// checks.
pub use suprnova_live::snapshot::MountedDocumentPath;
/// The fields of a snapshot schema: each field's category, codec and spec,
/// the schema itself, and whether state is validated as a reusable public
/// seed or a scoped instance. A [`SnapshotSchemaSet`], a field's
/// [`FieldMetadata`](crate::live::metadata::FieldMetadata) and a
/// [`ComponentInstance`] name them.
pub use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema,
};
/// The failure, with its closed reason, the snapshot boundary reports, as
/// [`MountedDocumentPath::parse`] returns it and a [`MountFailure`]
/// carries its kind, and the state, memo and mount schemas a
/// [`ComponentDescriptor`] binds.
pub use suprnova_live::snapshot::{SnapshotError, SnapshotErrorKind, SnapshotSchemaSet};
/// How a component field binds to the browser: its typed codec and the
/// issue a conversion reports, its synchronization timing and its URL
/// binding, each with the failure its constructors report. A field's
/// [`FieldMetadata`](crate::live::metadata::FieldMetadata) records them.
pub use suprnova_live::state::{
    BindingIssue, BindingIssueKind, BindingTiming, ModelCodec, TimingError, TimingErrorKind,
    UrlBinding, UrlBindingMode, UrlError, UrlErrorKind,
};
/// A field of an object an agent sent that a codec, an action's arguments
/// or a component's mount parameters would refuse, with its closed reason,
/// and the JSON Schema dialect their schemas name: what a headless Live
/// tool reports field by field (AGENT-013).
pub use suprnova_live::state::{FieldIssue, FieldIssueKind, JSON_SCHEMA_DIALECT};
/// A batch of browser model proposals and what applying it involves: the
/// binding schema and its fields, the limits, each raw and proposed value,
/// the result of applying one, and the failure, with its closed reason, a
/// batch reports. A [`ComponentInstance`] binds the batch.
pub use suprnova_live::state::{
    ModelBindingSchema, ModelFieldBinding, ProposalApplication, ProposalBatch, ProposalError,
    ProposalErrorKind, ProposalLimitError, ProposalLimits, ProposedValue, RawModelProposal,
};
/// A validated model path and the failure, with its closed reason, a
/// malformed one reports, which a [`ValidationIssue`] is built from and
/// returns.
pub use suprnova_live::state::{ModelPath, PathError, PathErrorKind};
/// The SHA-256 checksum of an upload, which [`IntegrityEvidence`],
/// [`VerifyTransfer`] and an [`UploadInspection`] carry.
pub use suprnova_live::upload::UploadChecksum;
/// The Live engine's upload contract of one component field: its accepted
/// types, image dimension limits, replacement policy and scan policy. A
/// field's [`FieldMetadata`](crate::live::metadata::FieldMetadata) records
/// it.
pub use suprnova_live::upload::{
    AcceptedUploadType, ScanFailurePolicy, UploadDimensionLimits, UploadFieldPolicy,
    UploadMediaType, UploadReplacementPolicy, UploadScanPolicy,
};
/// What an [`UploadApplicationValidator`] and an [`UploadScanner`] read and
/// answer: the authoritative inspection of a quarantined upload and its
/// bounded content, the browser's untrusted display metadata, the detected
/// and authoritative types and the image dimensions, the application's
/// decision, and the safe reason a scanner or a validator rejects with.
/// [`ScanInput`] and the validator's `validate` hand them to an
/// application.
pub use suprnova_live::upload::{
    ApplicationValidationDecision, ApplicationValidationInput, AuthoritativeUploadType,
    ClientUploadMetadata, DetectedUploadType, MediaDimensions, ScanReason, UploadContent,
    UploadInspection,
};
pub use suprnova_live::upload::{
    BoundedHeaders, ChunkDisposition, ChunkReceipt, DirectPartReference, DirectTransferInstruction,
    DirectUploadProvider, DurableUpload, DurableUploadId, FailedFinalize, FinalizeRequest,
    FinalizeToken, IntegrityEvidence, PrepareTransfer, PreparedFinalize, QuarantineBytes,
    ReadUpload, ReportDirectPart, ScanDisposition, ScanInput, TransferDisposition,
    TransferInstruction, TransferMethod, TransferPlan, TrustedProviderOrigin, TrustedProviderUrl,
    UploadApplicationValidator, UploadError, UploadErrorKind, UploadFinalizer, UploadFuture,
    UploadHandle, UploadPart, UploadProvider, UploadScanner, VerifyTransfer,
};
/// What a finalize step carries: the validated upload and the transfer
/// grant scope it is bound to, the retry key, the revision of the upload
/// record and the stage a failed finalize stopped at, as a
/// [`FinalizeRequest`], a [`PreparedFinalize`] and a [`FailedFinalize`]
/// hand them to an [`UploadFinalizer`].
pub use suprnova_live::upload::{
    FinalizeFailureStage, TransferGrantScope, UploadIdempotencyKey, UploadRevision, ValidatedUpload,
};
/// The host port that authorizes upload controls, with the request it
/// reads and the decision it returns, which a [`HostCapabilities`] takes.
pub use suprnova_live::upload::{
    UploadAuthorizationDecision, UploadAuthorizationPort, UploadAuthorizationRequest,
    UploadControlKind,
};
pub use upload_host::LiveUploadHost;
pub use upload_policy::{
    UploadPolicy, UploadPolicyBuilder, UploadPolicyError, UploadReplacement, UploadScan,
    UploadScanFailure, UploadType,
};
pub use validation::{
    ErrorBag, ValidationIssue, ValidationMessageId, ValidationStatus, ValidationText,
};

// Host errors retain their actionable context in the server log, while Live's
// signed protocol exposes only the existing closed component failure category.
impl From<crate::FrameworkError> for ComponentError {
    fn from(error: crate::FrameworkError) -> Self {
        tracing::warn!(error = %error, "Live component failed");
        Self::application_failure()
    }
}
