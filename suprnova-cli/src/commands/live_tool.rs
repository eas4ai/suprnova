//! Client for the application's Live tooling helper.
//!
//! The CLI has no framework dependency, so every registry, checker, runtime,
//! and artifact fact comes from the generated application's console binary,
//! started as `__suprnova:live-tool --protocol <n> --operation <op>` through the
//! explicit-binary Cargo wrapper. The helper writes one JSON envelope per
//! stdout line; human and build output stays on stderr. This module owns the
//! transport side only: it validates version, sequence, identity, shape,
//! length, count, and digest, and fails closed on anything else. It never
//! interprets the manifest or component contracts, and it never echoes
//! stdout content into a message.

use std::fmt;
use std::io::BufRead;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Newest protocol version this CLI speaks. Version 2 carries every
/// configured Live limit by its `.env` key; nothing else changed.
pub const PROTOCOL_VERSION: u16 = 2;
/// Oldest protocol version this CLI still speaks, for applications on a
/// framework from before version 2: the CLI asks for the newest and falls
/// back when the helper answers in this one.
pub const MIN_PROTOCOL_VERSION: u16 = 1;
/// Hidden console command exposed by applications built on the framework.
pub const HELPER_COMMAND: &str = "__suprnova:live-tool";
/// Longest encoded envelope line, including its newline.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Most bytes one helper run may write to stdout.
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Most envelopes one helper run may write.
pub const MAX_ENVELOPES: usize = 8192;
/// Most diagnostics one check may report.
pub const MAX_DIAGNOSTICS: usize = 2048;
/// Most components one run may report.
pub const MAX_COMPONENTS: usize = 1024;
/// Most asset envelopes one run may report.
pub const MAX_ASSETS: usize = 16;
/// Most decoded asset bytes one run may report.
pub const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;
/// Longest text any field other than asset content may carry.
pub const MAX_TEXT_BYTES: usize = 256;
/// Most configured limits one runtime report carries.
pub const MAX_LIMITS: usize = 64;
/// Longest asset file name.
pub const MAX_FILE_NAME_BYTES: usize = 128;

/// Operations the helper performs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Check every registered component view.
    Check,
    /// Report safe runtime and component metadata.
    Inspect,
    /// Export the reviewed artifacts.
    Assets,
}

impl Operation {
    /// Wire spelling of the operation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Inspect => "inspect",
            Self::Assets => "assets",
        }
    }
}

/// Outcome carried by the end marker.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The operation completed.
    Ok,
    /// The helper failed closed.
    #[default]
    Failed,
}

/// Checker severity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The contract is invalid.
    Error,
    /// The checker makes no proof claim.
    Unproved,
}

impl Severity {
    /// Human spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Unproved => "unproved",
        }
    }
}

/// Which reviewed file an asset envelope carries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// The typed manifest.
    Manifest,
    /// One runtime artifact.
    Artifact,
    /// One boot script.
    Boot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    protocol: u16,
    sequence: u32,
    operation: Operation,
    framework: String,
    assets: Option<String>,
    body: Body,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Body {
    Begin,
    Diagnostic(DiagnosticReport),
    Component(ComponentReport),
    Runtime(RuntimeReport),
    Summary(CheckSummary),
    Asset(AssetEnvelope),
    End(EndReport),
}

/// One checker diagnostic.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    /// Diagnosed component, when attributed.
    pub component: Option<String>,
    /// Diagnosed template identity, when attributed.
    pub view: Option<String>,
    /// Stable checker code.
    pub code: String,
    /// Error or unproved.
    pub severity: Severity,
    /// One-based line or zero.
    pub line: u32,
    /// One-based column or zero.
    pub column: u32,
}

/// Safe metadata for one component.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentReport {
    /// Registered name.
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
    /// Minimum wire protocol.
    pub minimum_protocol: u16,
    /// Declared fields.
    pub fields: u32,
    /// Fields with an upload policy.
    pub upload_fields: u32,
    /// Declared actions.
    pub actions: u32,
    /// Declared events.
    pub events: u32,
    /// Declared effects.
    pub effects: u32,
    /// Declared subscriptions.
    pub subscriptions: u32,
    /// Whether promotion re-renders.
    pub refresh_on_promote: bool,
    /// Short hex contract digest.
    pub contract_digest: String,
}

/// The configured Live limits, in the shape the exchange's protocol knows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConfigReport {
    /// Protocol 2: every configured limit by its `.env` key.
    Limits {
        /// One entry per `LIVE_*` limit key, in the order the manual lists
        /// them.
        limits: Vec<LimitReport>,
    },
    /// Protocol 1: the three limits that version reported.
    Legacy {
        /// Largest accepted request body.
        max_request_bytes: u64,
        /// Largest produced response body.
        max_response_bytes: u64,
        /// Longest request context lifetime.
        max_context_lifetime_ms: u64,
    },
}

/// One configured Live limit, by the `.env` key that sets it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LimitReport {
    /// The `.env` setting, such as `LIVE_MAX_HTML_BYTES`. Named `setting`,
    /// not `key`, so nothing in an inspection reads as key material.
    pub setting: String,
    /// The configured value.
    pub value: u64,
    /// The unit the value is counted in, such as `bytes`.
    pub unit: String,
}

/// Installed upload capabilities, by presence only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadHostReport {
    /// Upload host installed at all.
    pub installed: bool,
    /// Finalizer installed.
    pub finalizer: bool,
    /// Direct provider installed.
    pub direct_provider: bool,
    /// Scanner installed.
    pub scanner: bool,
    /// Application validator installed.
    pub application_validator: bool,
}

/// Assembled runtime services, by presence only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessReport {
    /// Clock service.
    pub clock: bool,
    /// Random service.
    pub random: bool,
    /// Key ring.
    pub key_ring: bool,
    /// Revision ledger.
    pub ledger: bool,
    /// Promotion service.
    pub promotion: bool,
    /// Execution service.
    pub execution: bool,
    /// Context validator.
    pub context_validator: bool,
    /// Host ports.
    pub host_ports: bool,
    /// Upload ports.
    pub upload_ports: bool,
    /// Upload services.
    pub upload_services: bool,
    /// Mount catalog.
    pub mount_catalog: bool,
    /// Response and cancellation ports.
    pub response_and_cancellation: bool,
    /// Subscription ports.
    pub subscription_ports: bool,
    /// Asynchronous state.
    pub async_state: bool,
}

/// Safe runtime state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeReport {
    /// A Live registry is bound.
    pub registry_bound: bool,
    /// Registered components.
    pub components: u32,
    /// Configured limits.
    pub config: ConfigReport,
    /// Upload capabilities.
    pub upload_host: UploadHostReport,
    /// The runtime assembled.
    pub runtime_bound: bool,
    /// Assembled services, when bound.
    pub readiness: Option<ReadinessReport>,
    /// Reviewed artifact identity.
    pub asset_identity: Option<String>,
    /// Embedded browser runtime version.
    pub browser_runtime_version: String,
    /// Browser runtime contract version.
    pub runtime_contract_version: u16,
    /// Served wire protocol versions.
    pub protocol_versions: Vec<u16>,
}

/// Totals for one check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSummary {
    /// A Live registry is bound.
    pub registry_bound: bool,
    /// Components checked.
    pub components: u32,
    /// Components fully proved.
    pub proved: u32,
    /// Error diagnostics.
    pub errors: u32,
    /// Unproved diagnostics.
    pub unproved: u32,
    /// Template files loaded.
    pub template_files: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetEnvelope {
    kind: AssetKind,
    file: String,
    bytes: u64,
    sha256: String,
    sri: String,
    content_type: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndReport {
    status: Outcome,
    error: Option<String>,
}

/// One reviewed file, decoded and digest-verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedAsset {
    /// Which reviewed file this is.
    pub kind: AssetKind,
    /// Validated single-component file name.
    pub file: String,
    /// Decoded bytes.
    pub bytes: Vec<u8>,
    /// Verified lowercase hex SHA-256.
    pub sha256: String,
    /// Verified subresource integrity value.
    pub sri: String,
    /// Content type to serve with.
    pub content_type: String,
}

/// Everything one successful protocol exchange produced.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Session {
    /// Framework version the helper reported.
    pub framework: String,
    /// Reviewed artifact identity, when the application's artifacts validate.
    pub assets: Option<String>,
    /// Outcome from the end marker.
    pub outcome: Outcome,
    /// Failure kind from the end marker.
    pub error: Option<String>,
    /// Diagnostics in helper order.
    pub diagnostics: Vec<DiagnosticReport>,
    /// Components in helper order.
    pub components: Vec<ComponentReport>,
    /// Runtime report, for inspect.
    pub runtime: Option<RuntimeReport>,
    /// Check summary, for check.
    pub summary: Option<CheckSummary>,
    /// Verified assets, for assets.
    pub assets_out: Vec<PublishedAsset>,
}

/// Why the exchange failed. Messages never include stdout content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolFailure {
    /// `cargo run` could not start.
    Spawn(String),
    /// Stdout could not be read.
    Read(String),
    /// The helper did not finish in time.
    Timeout(u64),
    /// The console exited without speaking the protocol.
    MissingHelper(Option<i32>),
    /// The console produced no output and exited cleanly.
    NoOutput,
    /// The console exited with a failure after a successful exchange.
    ChildFailed(Option<i32>),
    /// A line was not a protocol envelope.
    UnexpectedStdout { line: usize, bytes: usize },
    /// The helper speaks another protocol.
    UnsupportedProtocol(u16),
    /// The helper answered another operation.
    WrongOperation,
    /// Sequence numbers are not contiguous.
    OutOfSequence(usize),
    /// Framework or asset identity changed mid-stream.
    StaleIdentity(usize),
    /// The first envelope is not the begin marker.
    MissingBegin,
    /// A begin marker appeared again.
    RepeatedBegin(usize),
    /// Output ended before the end marker.
    MissingEnd,
    /// Output continued after the end marker.
    TrailingAfterEnd,
    /// A line exceeded the per-line cap.
    LineTooLong(usize),
    /// Output exceeded the total cap.
    OutputTooLarge,
    /// Too many envelopes.
    TooManyEnvelopes,
    /// A text field exceeded the text cap.
    TextTooLong(usize),
    /// Too many diagnostics.
    TooManyDiagnostics,
    /// Too many components.
    TooManyComponents,
    /// Too many assets.
    TooManyAssets,
    /// Decoded assets exceed the byte cap.
    AssetsTooLarge,
    /// A report kind that may appear once appeared twice.
    Duplicate(&'static str),
    /// An asset file name is unsafe.
    InvalidFileName(usize),
    /// An asset's content is not standard base64.
    AssetEncoding(String),
    /// An asset's decoded length differs from its declared length.
    AssetLength(String),
    /// An asset's digest or integrity value does not match its bytes.
    DigestMismatch(String),
    /// A configured limit carries a malformed key or unit, or too many came.
    InvalidLimit(usize),
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "Failed to start `cargo run --bin console`: {error}"),
            Self::Read(error) => {
                write!(f, "Failed to read the application helper's output: {error}")
            }
            Self::Timeout(seconds) => write!(
                f,
                "The application helper did not finish within {seconds}s; pass --timeout-secs to allow a longer build"
            ),
            Self::MissingHelper(code) => write!(
                f,
                "The application's console did not expose the Live tooling helper (exit {}, no protocol output); the application must depend on suprnova {} or later and its console binary must build",
                exit_label(*code),
                env!("CARGO_PKG_VERSION")
            ),
            Self::NoOutput => f.write_str("The application helper produced no output"),
            Self::ChildFailed(code) => write!(
                f,
                "The application helper exited with {} after completing the exchange",
                exit_label(*code)
            ),
            Self::UnexpectedStdout { line, bytes } => write!(
                f,
                "Unexpected or malformed output on stdout at line {line} ({bytes} bytes); the application helper prints only protocol envelopes"
            ),
            Self::UnsupportedProtocol(protocol) if *protocol > PROTOCOL_VERSION => write!(
                f,
                "The application helper speaks protocol {protocol}, newer than this CLI's protocols {MIN_PROTOCOL_VERSION} to {PROTOCOL_VERSION}; upgrade the suprnova CLI to the application's suprnova version"
            ),
            Self::UnsupportedProtocol(protocol) => write!(
                f,
                "The application helper speaks protocol {protocol}, older than this CLI's protocols {MIN_PROTOCOL_VERSION} to {PROTOCOL_VERSION}; upgrade the application's suprnova dependency to {} or later",
                env!("CARGO_PKG_VERSION")
            ),
            Self::WrongOperation => {
                f.write_str("The application helper answered a different operation than requested")
            }
            Self::OutOfSequence(line) => write!(f, "Envelope {line} is out of sequence"),
            Self::StaleIdentity(line) => write!(
                f,
                "Envelope {line} carries a different framework or asset identity than the first; the build is stale or mixed"
            ),
            Self::MissingBegin => f.write_str("The first envelope is not the begin marker"),
            Self::RepeatedBegin(line) => write!(f, "Envelope {line} repeats the begin marker"),
            Self::MissingEnd => f.write_str("The helper output ended without an end marker"),
            Self::TrailingAfterEnd => f.write_str("Output continued after the end marker"),
            Self::LineTooLong(line) => {
                write!(
                    f,
                    "Envelope {line} exceeds the {MAX_LINE_BYTES}-byte line cap"
                )
            }
            Self::OutputTooLarge => {
                write!(
                    f,
                    "The helper output exceeds the {MAX_TOTAL_BYTES}-byte cap"
                )
            }
            Self::TooManyEnvelopes => {
                write!(f, "The helper wrote more than {MAX_ENVELOPES} envelopes")
            }
            Self::TextTooLong(line) => write!(
                f,
                "Envelope {line} carries a field that is too long (over {MAX_TEXT_BYTES} bytes)"
            ),
            Self::TooManyDiagnostics => {
                write!(
                    f,
                    "The helper reported more than {MAX_DIAGNOSTICS} diagnostics"
                )
            }
            Self::TooManyComponents => {
                write!(
                    f,
                    "The helper reported more than {MAX_COMPONENTS} components"
                )
            }
            Self::TooManyAssets => write!(f, "The helper reported more than {MAX_ASSETS} assets"),
            Self::AssetsTooLarge => {
                write!(
                    f,
                    "The reported assets exceed {MAX_ASSET_BYTES} decoded bytes"
                )
            }
            Self::Duplicate(what) => write!(f, "The helper reported a duplicate {what}"),
            Self::InvalidFileName(line) => {
                write!(f, "Envelope {line} carries an unsafe asset file name")
            }
            Self::AssetEncoding(file) => write!(f, "Asset {file} is not valid standard base64"),
            Self::AssetLength(file) => {
                write!(
                    f,
                    "Asset {file} decoded to a different length than declared"
                )
            }
            Self::DigestMismatch(file) => write!(
                f,
                "Asset {file} does not match its declared SHA-256 digest or integrity value"
            ),
            Self::InvalidLimit(line) => write!(
                f,
                "Envelope {line} reports a configured limit with a malformed key or unit, or more than {MAX_LIMITS} limits"
            ),
        }
    }
}

fn exit_label(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_owned(), |code| code.to_string())
}

fn text_ok(value: &str) -> bool {
    value.len() <= MAX_TEXT_BYTES
}

fn optional_text_ok(value: Option<&String>) -> bool {
    value.is_none_or(|value| text_ok(value))
}

/// A limit's key is a `LIVE_*` configuration key and its unit a short plain
/// word or two, so neither can carry anything to the terminal but a name.
fn limit_ok(limit: &LimitReport) -> bool {
    limit.setting.len() <= 64
        && limit.setting.starts_with("LIVE_")
        && limit
            .setting
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !limit.unit.is_empty()
        && limit.unit.len() <= 32
        && limit
            .unit
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b' ')
}

fn file_name_ok(name: &str) -> bool {
    let mut bytes = name.bytes();
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME_BYTES
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Reads and validates one complete protocol exchange.
///
/// Every cap, the sequence, the identity, the marker order, and every asset
/// digest are checked here; the first violation ends the exchange.
/// Escape control characters in helper-provided text before it reaches the
/// terminal, and bound its length; the helper is trusted code, but its
/// strings quote template and component names an application controls.
pub(crate) fn display_text(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_control() {
                ch.escape_default().to_string()
            } else {
                ch.to_string()
            }
        })
        .take(200)
        .collect()
}

/// Reads and validates one complete exchange in `protocol`.
pub fn consume_protocol<R: BufRead>(
    reader: R,
    operation: Operation,
    protocol: u16,
) -> Result<Session, ToolFailure> {
    let mut reader = reader.take((MAX_TOTAL_BYTES + 1) as u64);
    let mut buffer = Vec::new();
    let mut session = Session::default();
    let mut total = 0usize;
    let mut line = 0usize;
    let mut identity: Option<(String, Option<String>)> = None;
    let mut ended = false;
    let mut asset_bytes = 0usize;
    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .map_err(|error| ToolFailure::Read(error.to_string()))?;
        if read == 0 {
            break;
        }
        line += 1;
        total += read;
        if total > MAX_TOTAL_BYTES {
            return Err(ToolFailure::OutputTooLarge);
        }
        if read > MAX_LINE_BYTES {
            return Err(ToolFailure::LineTooLong(line));
        }
        if line > MAX_ENVELOPES {
            return Err(ToolFailure::TooManyEnvelopes);
        }
        if ended {
            return Err(ToolFailure::TrailingAfterEnd);
        }
        let text = buffer.strip_suffix(b"\n").unwrap_or(&buffer);
        let envelope: Envelope = serde_json::from_slice(text)
            .map_err(|_| ToolFailure::UnexpectedStdout { line, bytes: read })?;
        if envelope.protocol != protocol {
            return Err(ToolFailure::UnsupportedProtocol(envelope.protocol));
        }
        if envelope.operation != operation {
            return Err(ToolFailure::WrongOperation);
        }
        if envelope.sequence as usize != line - 1 {
            return Err(ToolFailure::OutOfSequence(line));
        }
        if !text_ok(&envelope.framework) || !optional_text_ok(envelope.assets.as_ref()) {
            return Err(ToolFailure::TextTooLong(line));
        }
        match &identity {
            None => {
                if !matches!(envelope.body, Body::Begin) {
                    return Err(ToolFailure::MissingBegin);
                }
                identity = Some((envelope.framework.clone(), envelope.assets.clone()));
                session.framework = envelope.framework;
                session.assets = envelope.assets;
                continue;
            }
            Some((framework, assets)) => {
                if *framework != envelope.framework || *assets != envelope.assets {
                    return Err(ToolFailure::StaleIdentity(line));
                }
            }
        }
        match envelope.body {
            Body::Begin => return Err(ToolFailure::RepeatedBegin(line)),
            Body::Diagnostic(report) => {
                if !optional_text_ok(report.component.as_ref())
                    || !optional_text_ok(report.view.as_ref())
                    || !text_ok(&report.code)
                {
                    return Err(ToolFailure::TextTooLong(line));
                }
                if session.diagnostics.len() >= MAX_DIAGNOSTICS {
                    return Err(ToolFailure::TooManyDiagnostics);
                }
                session.diagnostics.push(report);
            }
            Body::Component(report) => {
                if !text_ok(&report.name)
                    || !text_ok(&report.view)
                    || !text_ok(&report.contract_digest)
                {
                    return Err(ToolFailure::TextTooLong(line));
                }
                if session.components.len() >= MAX_COMPONENTS {
                    return Err(ToolFailure::TooManyComponents);
                }
                session.components.push(report);
            }
            Body::Runtime(report) => {
                if !optional_text_ok(report.asset_identity.as_ref())
                    || !text_ok(&report.browser_runtime_version)
                    || report.protocol_versions.len() > 16
                {
                    return Err(ToolFailure::TextTooLong(line));
                }
                // Each protocol has its one shape: protocol 1 the three named
                // limits, protocol 2 every limit by its key.
                let shape_ok = match &report.config {
                    ConfigReport::Limits { limits } => {
                        protocol >= 2 && limits.len() <= MAX_LIMITS && limits.iter().all(limit_ok)
                    }
                    ConfigReport::Legacy { .. } => protocol == 1,
                };
                if !shape_ok {
                    return Err(ToolFailure::InvalidLimit(line));
                }
                if session.runtime.replace(report).is_some() {
                    return Err(ToolFailure::Duplicate("runtime report"));
                }
            }
            Body::Summary(report) => {
                if session.summary.replace(report).is_some() {
                    return Err(ToolFailure::Duplicate("check summary"));
                }
            }
            Body::Asset(asset) => {
                if !file_name_ok(&asset.file) {
                    return Err(ToolFailure::InvalidFileName(line));
                }
                if !text_ok(&asset.sha256) || !text_ok(&asset.sri) || !text_ok(&asset.content_type)
                {
                    return Err(ToolFailure::TextTooLong(line));
                }
                if session.assets_out.len() >= MAX_ASSETS {
                    return Err(ToolFailure::TooManyAssets);
                }
                if session
                    .assets_out
                    .iter()
                    .any(|known| known.file == asset.file)
                {
                    return Err(ToolFailure::Duplicate("asset file"));
                }
                let bytes = BASE64
                    .decode(asset.content.as_bytes())
                    .map_err(|_| ToolFailure::AssetEncoding(asset.file.clone()))?;
                if bytes.len() as u64 != asset.bytes {
                    return Err(ToolFailure::AssetLength(asset.file));
                }
                asset_bytes += bytes.len();
                if asset_bytes > MAX_ASSET_BYTES {
                    return Err(ToolFailure::AssetsTooLarge);
                }
                let digest = Sha256::digest(&bytes);
                let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                let sri = format!("sha256-{}", BASE64.encode(digest));
                if hex != asset.sha256 || sri != asset.sri {
                    return Err(ToolFailure::DigestMismatch(asset.file));
                }
                session.assets_out.push(PublishedAsset {
                    kind: asset.kind,
                    file: asset.file,
                    bytes,
                    sha256: hex,
                    sri,
                    content_type: asset.content_type,
                });
            }
            Body::End(report) => {
                if !optional_text_ok(report.error.as_ref()) {
                    return Err(ToolFailure::TextTooLong(line));
                }
                session.outcome = report.status;
                session.error = report.error;
                ended = true;
            }
        }
    }
    if line == 0 {
        return Err(ToolFailure::NoOutput);
    }
    if !ended {
        return Err(ToolFailure::MissingEnd);
    }
    Ok(session)
}

/// Runs the application helper for `operation` and consumes its exchange.
///
/// Stderr is inherited so build output stays visible; stdout is read on a
/// worker thread under `timeout`, after which the child is killed.
pub fn run(
    operation: Operation,
    extra_args: &[String],
    timeout: Duration,
) -> Result<Session, ToolFailure> {
    match run_in(PROTOCOL_VERSION, operation, extra_args, timeout) {
        // A helper from before protocol 2 answers in protocol 1 and refuses
        // the request; it still speaks protocol 1, so ask again in that.
        Err(ToolFailure::UnsupportedProtocol(spoken)) if spoken == MIN_PROTOCOL_VERSION => {
            run_in(MIN_PROTOCOL_VERSION, operation, extra_args, timeout)
        }
        result => result,
    }
}

/// Runs the helper once, asking for `protocol`.
fn run_in(
    protocol: u16,
    operation: Operation,
    extra_args: &[String],
    timeout: Duration,
) -> Result<Session, ToolFailure> {
    let protocol_arg = protocol.to_string();
    let mut args: Vec<&str> = vec![
        HELPER_COMMAND,
        "--protocol",
        &protocol_arg,
        "--operation",
        operation.as_str(),
    ];
    args.extend(extra_args.iter().map(String::as_str));
    exchange(
        super::cargo_run_console(&args),
        operation,
        protocol,
        timeout,
    )
}

/// Spawns `command` as the helper and consumes its exchange for
/// `operation` in `protocol`, under `timeout`.
fn exchange(
    mut command: std::process::Command,
    operation: Operation,
    protocol: u16,
    timeout: Duration,
) -> Result<Session, ToolFailure> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    // Its own process group, so a timeout reaches every process the helper
    // starts (ROOT-38): the helper is `cargo run`, and the console binary
    // it runs is a process of its own.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command
        .spawn()
        .map_err(|error| ToolFailure::Spawn(error.to_string()))?;
    // The group no longer receives the terminal's Ctrl+C, so it is
    // forwarded for as long as the helper runs.
    #[cfg(unix)]
    let _interrupts = interrupt::forward_to(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolFailure::Spawn("stdout was not captured".to_owned()))?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let result = consume_protocol(std::io::BufReader::new(stdout), operation, protocol);
        let _ = sender.send(result);
    });
    let parsed = match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            kill_helper(&mut child);
            let _ = child.wait();
            // Never joined (ROOT-38). A process the helper started can
            // still hold the stdout pipe after the helper itself is killed,
            // and the reader blocked on that pipe would then hold this call
            // past its own timeout for as long as the stray process lives.
            // Detached, the reader ends when the last writer closes the pipe.
            drop(reader);
            return Err(ToolFailure::Timeout(timeout.as_secs()));
        }
    };
    if parsed.is_err() {
        // The exchange is already void; never wait on a child that may keep
        // writing into a closed pipe or otherwise refuse to exit.
        kill_helper(&mut child);
    }
    let status = child
        .wait()
        .map_err(|error| ToolFailure::Read(error.to_string()))?;
    let _ = reader.join();
    match parsed {
        Ok(session) => {
            if !status.success() && session.outcome == Outcome::Ok {
                return Err(ToolFailure::ChildFailed(status.code()));
            }
            Ok(session)
        }
        Err(ToolFailure::NoOutput) if !status.success() => {
            Err(ToolFailure::MissingHelper(status.code()))
        }
        Err(error) => Err(error),
    }
}

/// Kills the helper and, on Unix, every process in its group (ROOT-38).
/// Killing the helper alone left the console binary `cargo run` started
/// running after the call had returned.
fn kill_helper(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(group) = i32::try_from(child.id()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(group),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
}

/// Forwards the terminal's Ctrl+C to the helper groups that are running.
///
/// A helper in its own process group is outside the terminal's foreground
/// group, so Ctrl+C reaches only this process. The handler forwards SIGINT
/// to every running helper group, then exits with the interrupt status,
/// as the default disposition would have. It is installed once and stays
/// for the life of the process; with no helper running it only exits. A
/// process that already installed its own handler keeps it, and its helpers
/// then get no forwarded interrupt.
#[cfg(unix)]
mod interrupt {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    static GROUPS: Mutex<BTreeSet<i32>> = Mutex::new(BTreeSet::new());
    static INSTALLED: OnceLock<bool> = OnceLock::new();

    /// Keeps one helper group registered for forwarding until dropped.
    pub(super) struct Forwarding(Option<i32>);

    /// Registers the group `pid` leads, installing the handler on first use.
    pub(super) fn forward_to(pid: u32) -> Forwarding {
        INSTALLED.get_or_init(|| {
            ctrlc::set_handler(|| {
                forward_interrupt();
                std::process::exit(130);
            })
            .is_ok()
        });
        let group = i32::try_from(pid).ok();
        if let Some(group) = group {
            groups().insert(group);
        }
        Forwarding(group)
    }

    impl Drop for Forwarding {
        fn drop(&mut self) {
            if let Some(group) = self.0 {
                groups().remove(&group);
            }
        }
    }

    /// Sends SIGINT to every registered helper group.
    fn forward_interrupt() {
        interrupt_groups(&groups());
    }

    /// Sends SIGINT to each group in `targets`.
    pub(super) fn interrupt_groups(targets: &BTreeSet<i32>) {
        for group in targets {
            let _ = killpg(Pid::from_raw(*group), Signal::SIGINT);
        }
    }

    /// Whether the group `pid` leads is registered for forwarding.
    #[cfg(test)]
    pub(super) fn is_registered(pid: u32) -> bool {
        i32::try_from(pid).is_ok_and(|group| groups().contains(&group))
    }

    fn groups() -> MutexGuard<'static, BTreeSet<i32>> {
        GROUPS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ROOT-38: the helper runs in its own process group, so the terminal's
    /// Ctrl+C no longer reaches it directly. A running helper's group is
    /// registered for the forwarder, which sends it SIGINT, and is dropped
    /// from it when the helper is done. The signal goes only to this test's
    /// own group, so helpers other tests run at the same time are untouched.
    #[cfg(unix)]
    #[test]
    fn an_interrupt_is_forwarded_to_the_helper_group_while_it_runs() {
        use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

        let mut command = std::process::Command::new("sleep");
        command.arg("30").process_group(0);
        let mut child = command.spawn().expect("spawn a helper stand-in");
        let forwarding = interrupt::forward_to(child.id());
        assert!(interrupt::is_registered(child.id()));

        let group = i32::try_from(child.id()).expect("a process id");
        interrupt::interrupt_groups(&std::collections::BTreeSet::from([group]));
        // Unregistered before the stand-in is reaped, so its process id
        // cannot have been reused by another test's helper yet.
        drop(forwarding);
        assert!(
            !interrupt::is_registered(child.id()),
            "a finished helper is no longer forwarded to"
        );
        let status = child.wait().expect("wait for the helper stand-in");
        assert_eq!(status.signal(), Some(nix::libc::SIGINT));
    }

    /// ROOT-38: the helper timeout bounds the call even when a process the
    /// helper started still holds the stdout pipe after the helper itself is
    /// killed. The reader blocked on that pipe used to be joined
    /// unconditionally on timeout, so the call waited for the stray process
    /// rather than for its own timeout. The shell here starts a sleeper that
    /// inherits its stdout, then becomes a sleeper itself and never answers.
    #[cfg(unix)]
    #[test]
    fn the_helper_timeout_bounds_the_call_when_a_child_of_the_helper_keeps_stdout_open() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & exec sleep 30"]);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let result = exchange(
                command,
                Operation::Check,
                PROTOCOL_VERSION,
                Duration::from_millis(200),
            );
            let _ = done_tx.send(matches!(result, Err(ToolFailure::Timeout(_))));
        });
        let timed_out = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the call returns within its own timeout, not when the stray process exits");
        assert!(timed_out, "the call reports the helper timeout");
    }

    /// ROOT-38: a helper timeout kills every process the helper started,
    /// not only the helper itself. A process the helper left behind kept
    /// running after the call had returned, holding the stdout pipe and
    /// whatever work it was blocked in. The shell records the sleeper it
    /// starts, then becomes a sleeper itself and never answers.
    #[cfg(unix)]
    #[test]
    fn the_helper_timeout_kills_every_process_the_helper_started() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("sleeper.pid");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & echo $! > \"$1\"; exec sleep 30", "sh"]);
        command.arg(&pid_file);
        let result = exchange(
            command,
            Operation::Check,
            PROTOCOL_VERSION,
            Duration::from_millis(300),
        );
        assert!(matches!(result, Err(ToolFailure::Timeout(_))));
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("the helper recorded the process it started")
            .trim()
            .parse()
            .expect("a process id");
        let pid = nix::unistd::Pid::from_raw(pid);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while nix::sys::signal::kill(pid, None).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "the process the helper started is still running after the timeout"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn limits_carry_only_configuration_keys_and_plain_units() {
        let limit = |setting: &str, unit: &str| LimitReport {
            setting: setting.to_owned(),
            value: 1,
            unit: unit.to_owned(),
        };
        assert!(limit_ok(&limit("LIVE_MAX_HTML_BYTES", "bytes")));
        assert!(limit_ok(&limit("LIVE_MORPH_MAX_KEYS", "keyed elements")));
        assert!(!limit_ok(&limit("APP_KEY", "bytes")));
        assert!(!limit_ok(&limit("LIVE_MAX_HTML_BYTES\u{1b}[2J", "bytes")));
        assert!(!limit_ok(&limit("LIVE_MAX_HTML_BYTES", "")));
        assert!(!limit_ok(&limit("LIVE_MAX_HTML_BYTES", "bytes\n")));
        assert!(!limit_ok(&limit(
            &format!("LIVE_{}", "A".repeat(60)),
            "bytes"
        )));
    }

    #[test]
    fn file_names_are_single_safe_components() {
        assert!(file_name_ok("suprnova-live.esm.js"));
        assert!(file_name_ok("suprnova-live.assets.json"));
        assert!(!file_name_ok(""));
        assert!(!file_name_ok(".hidden"));
        assert!(!file_name_ok("../x.js"));
        assert!(!file_name_ok("a/b.js"));
        assert!(!file_name_ok("a b.js"));
        assert!(!file_name_ok(&"a".repeat(MAX_FILE_NAME_BYTES + 1)));
    }
}
