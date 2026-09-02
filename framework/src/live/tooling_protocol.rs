//! Bounded, versioned JSON-lines protocol between the `suprnova` CLI and an
//! application's Live tooling helper.
//!
//! The CLI keeps no framework dependency. It starts the application's console
//! binary as `__suprnova:live-tool --protocol 1 --operation <check|inspect|assets>`
//! and reads one [`Envelope`](crate::live::tooling_protocol::Envelope) per
//! stdout line; human and build output stays on
//! stderr. Every envelope carries the protocol version, a contiguous sequence
//! number, the operation, the framework version, the reviewed artifact identity,
//! and one typed, redacted body. The first body is
//! [`Body::Begin`](crate::live::tooling_protocol::Body::Begin) and the last is
//! [`Body::End`](crate::live::tooling_protocol::Body::End), which carries the
//! outcome. The caps below bound what one
//! helper run may write; the CLI enforces the same caps while reading and fails
//! closed on anything unsupported, truncated, or oversized.
//!
//! Only closed enumerations, bounded integers, validated identities, content
//! digests, and base64 artifact bytes cross this boundary. No state, key
//! material, credential, cookie, or request body ever does.

use serde::{Deserialize, Serialize};

/// Protocol version spoken by this framework build.
pub const PROTOCOL_VERSION: u16 = 1;
/// Console command name of the hidden application helper.
pub const COMMAND_NAME: &str = "__suprnova:live-tool";
/// Longest encoded envelope line, including its newline.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Most bytes one helper run writes to stdout.
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Most envelopes one helper run writes, end marker included.
pub const MAX_ENVELOPES: usize = 8192;
/// Most diagnostics one check reports before failing closed.
pub const MAX_DIAGNOSTICS: usize = 2048;
/// Most components one run reports before failing closed.
pub const MAX_COMPONENTS: usize = 1024;
/// Most asset envelopes one run reports.
pub const MAX_ASSETS: usize = 16;
/// Most decoded asset bytes one run reports.
pub const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;
/// Most template roots one check accepts.
pub const MAX_TEMPLATE_ROOTS: usize = 8;
/// Most template files one check loads.
pub const MAX_TEMPLATE_FILES: usize = 512;
/// Largest template file one check loads.
pub const MAX_TEMPLATE_FILE_BYTES: usize = 1024 * 1024;
/// Most template bytes one check loads across every root.
pub const MAX_TEMPLATE_TOTAL_BYTES: usize = 16 * 1024 * 1024;
/// Deepest directory nesting one check walks below a template root.
pub const MAX_TEMPLATE_DEPTH: usize = 16;
/// Longest text any field other than asset content carries.
pub const MAX_TEXT_BYTES: usize = 256;

/// One of the closed set of helper operations.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Check every registered component view against the integrated checker.
    Check,
    /// Report safe runtime, configuration, provider, and component metadata.
    Inspect,
    /// Export the reviewed artifact manifest, artifacts, and boot scripts.
    Assets,
}

impl Operation {
    /// Returns the wire spelling of the operation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Inspect => "inspect",
            Self::Assets => "assets",
        }
    }

    /// Parses the wire spelling of an operation.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "check" => Some(Self::Check),
            "inspect" => Some(Self::Inspect),
            "assets" => Some(Self::Assets),
            _ => None,
        }
    }
}

/// Outcome carried by the end marker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The operation completed; check diagnostics may still be present.
    Ok,
    /// The operation failed closed; `error` names the failure kind.
    Failed,
}

/// Checker severity as carried on the wire.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The checked contract is invalid.
    Error,
    /// The checker makes no proof claim for a dynamic structure.
    Unproved,
}

/// Which reviewed file an asset envelope carries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// The typed artifact manifest.
    Manifest,
    /// One reviewed runtime artifact.
    Artifact,
    /// One framework boot script.
    Boot,
}

/// One stdout line of the helper protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    /// Always [`PROTOCOL_VERSION`] for this framework build.
    pub protocol: u16,
    /// Contiguous position of this envelope within the run, starting at zero.
    pub sequence: u32,
    /// The operation the helper was asked to perform.
    pub operation: Operation,
    /// The framework crate version that produced the envelope.
    pub framework: String,
    /// The reviewed artifact identity, or `None` when the artifacts fail validation.
    pub assets: Option<String>,
    /// The typed, redacted body.
    pub body: Body,
}

/// Typed envelope bodies.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Body {
    /// First envelope of every run.
    Begin,
    /// One checker diagnostic.
    Diagnostic(DiagnosticReport),
    /// Safe metadata for one registered component.
    Component(ComponentReport),
    /// Safe runtime, configuration, and provider state.
    Runtime(RuntimeReport),
    /// Totals for one check run, emitted after every diagnostic.
    Summary(CheckSummary),
    /// One reviewed file with its bytes and digest.
    Asset(AssetReport),
    /// Last envelope of every run.
    End(EndReport),
}

/// One checker diagnostic without template text.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    /// The diagnosed component, when the checker attributes one.
    pub component: Option<String>,
    /// The diagnosed template identity, when the checker attributes one.
    pub view: Option<String>,
    /// Stable checker diagnostic code.
    pub code: String,
    /// Whether the contract is invalid or merely unproved.
    pub severity: Severity,
    /// One-based source line, or zero when unknown.
    pub line: u32,
    /// One-based source column, or zero when unknown.
    pub column: u32,
}

/// Safe metadata for one registered component.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentReport {
    /// Registered component name.
    pub name: String,
    /// Declared template identity.
    pub view: String,
    /// Component contract version.
    pub component_version: u16,
    /// State schema version.
    pub state_schema_version: u16,
    /// Action schema version.
    pub action_schema_version: u16,
    /// Checker contract version.
    pub checker_contract_version: u16,
    /// Minimum wire protocol the component accepts.
    pub minimum_protocol: u16,
    /// Number of declared fields.
    pub fields: u32,
    /// Number of fields carrying an upload policy.
    pub upload_fields: u32,
    /// Number of declared actions.
    pub actions: u32,
    /// Number of declared events.
    pub events: u32,
    /// Number of declared effects.
    pub effects: u32,
    /// Number of declared subscriptions.
    pub subscriptions: u32,
    /// Whether promotion re-renders the component.
    pub refresh_on_promote: bool,
    /// First eight bytes of the contract digest, hex encoded.
    pub contract_digest: String,
}

/// Configured Live byte and lifetime limits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigReport {
    /// Largest accepted Live request body.
    pub max_request_bytes: u64,
    /// Largest produced Live response body.
    pub max_response_bytes: u64,
    /// Longest trusted request context lifetime.
    pub max_context_lifetime_ms: u64,
}

/// Which application upload capabilities are installed, by presence only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadHostReport {
    /// Whether the application installed a Live upload host at all.
    pub installed: bool,
    /// Whether a finalizer is installed.
    pub finalizer: bool,
    /// Whether a direct upload provider is installed.
    pub direct_provider: bool,
    /// Whether a scanner is installed.
    pub scanner: bool,
    /// Whether an application validator is installed.
    pub application_validator: bool,
}

/// Which runtime services assembled, by presence only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessReport {
    /// Clock service.
    pub clock: bool,
    /// Random service.
    pub random: bool,
    /// Snapshot key ring.
    pub key_ring: bool,
    /// Revision ledger.
    pub ledger: bool,
    /// Seed promotion service.
    pub promotion: bool,
    /// Execution service.
    pub execution: bool,
    /// Request context validator.
    pub context_validator: bool,
    /// Host ports.
    pub host_ports: bool,
    /// Upload ports.
    pub upload_ports: bool,
    /// Upload services.
    pub upload_services: bool,
    /// Mount catalog.
    pub mount_catalog: bool,
    /// Response intent and cancellation ports.
    pub response_and_cancellation: bool,
    /// Subscription ports.
    pub subscription_ports: bool,
    /// Asynchronous update state.
    pub async_state: bool,
}

/// Safe runtime, configuration, provider, and artifact state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeReport {
    /// Whether a Live registry is bound in the application container.
    pub registry_bound: bool,
    /// Number of registered components.
    pub components: u32,
    /// Configured limits.
    pub config: ConfigReport,
    /// Installed upload capabilities.
    pub upload_host: UploadHostReport,
    /// Whether the Live runtime assembled from the bound services.
    pub runtime_bound: bool,
    /// Assembled runtime services, when the runtime is bound.
    pub readiness: Option<ReadinessReport>,
    /// The reviewed artifact identity, when the artifacts validate.
    pub asset_identity: Option<String>,
    /// The embedded browser runtime version.
    pub browser_runtime_version: String,
    /// The browser runtime contract version.
    pub runtime_contract_version: u16,
    /// Wire protocol versions the framework serves.
    pub protocol_versions: Vec<u16>,
}

/// Totals for one check run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSummary {
    /// Whether a Live registry is bound in the application container.
    pub registry_bound: bool,
    /// Number of registered components checked.
    pub components: u32,
    /// Number of components whose every static contract was proved.
    pub proved: u32,
    /// Number of error diagnostics.
    pub errors: u32,
    /// Number of unproved diagnostics.
    pub unproved: u32,
    /// Number of template files loaded from the given roots.
    pub template_files: u32,
}

/// One reviewed file with its bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetReport {
    /// Which reviewed file this is.
    pub kind: AssetKind,
    /// Fixed file name without any directory component.
    pub file: String,
    /// Decoded length in bytes.
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the decoded bytes.
    pub sha256: String,
    /// Subresource integrity value of the decoded bytes.
    pub sri: String,
    /// Content type to serve the file with.
    pub content_type: String,
    /// Standard base64 encoding of the bytes.
    pub content: String,
}

/// The end marker's outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndReport {
    /// Whether the operation completed.
    pub status: Outcome,
    /// The failure kind when `status` is [`Outcome::Failed`].
    pub error: Option<String>,
}
