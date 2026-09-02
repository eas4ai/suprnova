//! Hidden console helper that answers the `suprnova` CLI's Live tooling protocol.
//!
//! The CLI keeps no framework dependency, so every registry, checker, runtime,
//! and artifact fact it needs comes from this helper, which runs inside the
//! application after its ordinary console bootstrap, exactly like any other
//! console command. Stdout carries only the protocol in
//! [`crate::live::tooling_protocol`]; nothing here writes human text to
//! stdout.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest as _, Sha256};
use suprnova_live::SUPPORTED_PROTOCOL_VERSIONS;
use suprnova_live::artifacts::{
    ARTIFACT_CONTENT_TYPE, BROWSER_RUNTIME_VERSION, MANIFEST_FILE, RUNTIME_CONTRACT_VERSION,
};
use suprnova_live::checker::{CheckerLimits, DiagnosticSeverity, TemplateCatalog, TemplateChecker};
use suprnova_live::identity::{ComponentName, ViewName};
use suprnova_live::metadata::ComponentMetadata;

use super::assets::live_asset_catalog;
use super::config::LiveConfig;
use super::registry::LiveRegistry;
use super::runtime::LiveRuntime;
use super::tooling_protocol::{
    AssetKind, AssetReport, Body, COMMAND_NAME, CheckSummary, ComponentReport, ConfigReport,
    DiagnosticReport, EndReport, Envelope, MAX_ASSET_BYTES, MAX_ASSETS, MAX_COMPONENTS,
    MAX_DIAGNOSTICS, MAX_ENVELOPES, MAX_LINE_BYTES, MAX_TEMPLATE_DEPTH, MAX_TEMPLATE_FILE_BYTES,
    MAX_TEMPLATE_FILES, MAX_TEMPLATE_ROOTS, MAX_TEMPLATE_TOTAL_BYTES, MAX_TOTAL_BYTES, Operation,
    Outcome, PROTOCOL_VERSION, ReadinessReport, RuntimeReport, Severity, UploadHostReport,
};
use super::upload_host::LiveUploadHost;
use crate::App;
use crate::console::CommandEntry;
use crate::error::FrameworkError;

/// Bytes kept free so the end marker always fits under the total cap.
const END_MARKER_RESERVE: usize = 1024;
/// Content type the manifest is published with.
const MANIFEST_CONTENT_TYPE: &str = "application/json";

/// Why the helper failed closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ToolingErrorKind {
    /// The CLI asked for a protocol this framework does not speak.
    UnsupportedProtocol,
    /// The CLI asked for an operation this framework does not know.
    UnknownOperation,
    /// No Live registry is bound in the application container.
    RegistryUnavailable,
    /// A template root is missing, not a directory, or a symlink.
    TemplateRootRejected,
    /// A template entry is a symlink, unreadable, misnamed, or duplicated.
    TemplateRejected,
    /// The template roots, files, depth, or bytes exceed the protocol caps.
    TemplateLimitExceeded,
    /// More components are registered than the protocol reports.
    ComponentLimitExceeded,
    /// More diagnostics were produced than the protocol reports.
    DiagnosticLimitExceeded,
    /// The reviewed artifacts failed validation.
    AssetsUnavailable,
    /// The output would exceed a protocol cap.
    OutputLimitExceeded,
    /// Stdout could not be written.
    OutputFailed,
}

impl ToolingErrorKind {
    /// Returns the stable wire name of the failure.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedProtocol => "live_tooling_unsupported_protocol",
            Self::UnknownOperation => "live_tooling_unknown_operation",
            Self::RegistryUnavailable => "live_tooling_registry_unavailable",
            Self::TemplateRootRejected => "live_tooling_template_root_rejected",
            Self::TemplateRejected => "live_tooling_template_rejected",
            Self::TemplateLimitExceeded => "live_tooling_template_limit_exceeded",
            Self::ComponentLimitExceeded => "live_tooling_component_limit_exceeded",
            Self::DiagnosticLimitExceeded => "live_tooling_diagnostic_limit_exceeded",
            Self::AssetsUnavailable => "live_tooling_assets_unavailable",
            Self::OutputLimitExceeded => "live_tooling_output_limit_exceeded",
            Self::OutputFailed => "live_tooling_output_failed",
        }
    }
}

/// A closed helper failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ToolingError {
    kind: ToolingErrorKind,
}

impl ToolingError {
    const fn new(kind: ToolingErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the failure kind.
    #[must_use]
    pub const fn kind(self) -> ToolingErrorKind {
        self.kind
    }
}

impl fmt::Display for ToolingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ToolingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ToolingError {}

impl From<ToolingError> for FrameworkError {
    fn from(error: ToolingError) -> Self {
        Self::internal(format!("Live tooling helper failed: {error}"))
    }
}

/// One validated helper invocation.
#[derive(Clone, Debug)]
pub struct ToolRequest {
    protocol: u16,
    operation: Operation,
    template_roots: Vec<PathBuf>,
}

impl ToolRequest {
    /// Creates a request for a known operation; the protocol is checked at execution.
    #[must_use]
    pub const fn new(protocol: u16, operation: Operation, template_roots: Vec<PathBuf>) -> Self {
        Self {
            protocol,
            operation,
            template_roots,
        }
    }

    /// Parses the command line form of a request.
    pub fn parse(
        protocol: u16,
        operation: &str,
        template_roots: Vec<PathBuf>,
    ) -> Result<Self, ToolingError> {
        let operation = Operation::parse(operation)
            .ok_or(ToolingError::new(ToolingErrorKind::UnknownOperation))?;
        Ok(Self::new(protocol, operation, template_roots))
    }

    /// Returns the requested protocol version.
    #[must_use]
    pub const fn protocol(&self) -> u16 {
        self.protocol
    }

    /// Returns the requested operation.
    #[must_use]
    pub const fn operation(&self) -> Operation {
        self.operation
    }

    /// Returns the template roots a check reads.
    #[must_use]
    pub fn template_roots(&self) -> &[PathBuf] {
        &self.template_roots
    }
}

struct Emitter<'sink> {
    sink: &'sink mut dyn Write,
    operation: Operation,
    assets: Option<String>,
    sequence: u32,
    written: usize,
}

impl Emitter<'_> {
    fn emit(&mut self, body: Body) -> Result<(), ToolingError> {
        let is_end = matches!(body, Body::End(_));
        let envelope = Envelope {
            protocol: PROTOCOL_VERSION,
            sequence: self.sequence,
            operation: self.operation,
            framework: env!("CARGO_PKG_VERSION").to_owned(),
            assets: self.assets.clone(),
            body,
        };
        let mut line = serde_json::to_vec(&envelope)
            .map_err(|_| ToolingError::new(ToolingErrorKind::OutputFailed))?;
        line.push(b'\n');
        let reserve = if is_end { 0 } else { END_MARKER_RESERVE };
        let envelopes_after = self.sequence as usize + 1 + usize::from(!is_end);
        if line.len() > MAX_LINE_BYTES
            || self.written + line.len() + reserve > MAX_TOTAL_BYTES
            || envelopes_after > MAX_ENVELOPES
        {
            return Err(ToolingError::new(ToolingErrorKind::OutputLimitExceeded));
        }
        self.sink
            .write_all(&line)
            .map_err(|_| ToolingError::new(ToolingErrorKind::OutputFailed))?;
        self.written += line.len();
        self.sequence += 1;
        Ok(())
    }
}

/// Runs one helper operation, writing the complete protocol exchange to `sink`.
///
/// The begin marker and the end marker are always written, even when the
/// operation fails closed; the returned error mirrors the end marker.
pub fn execute(request: &ToolRequest, sink: &mut dyn Write) -> Result<(), ToolingError> {
    let assets = live_asset_catalog()
        .ok()
        .map(|catalog| catalog.identity().to_owned());
    let mut emitter = Emitter {
        sink,
        operation: request.operation,
        assets,
        sequence: 0,
        written: 0,
    };
    emitter.emit(Body::Begin)?;
    let outcome = if request.protocol == PROTOCOL_VERSION {
        match request.operation {
            Operation::Check => run_check(&mut emitter, &request.template_roots),
            Operation::Inspect => run_inspect(&mut emitter),
            Operation::Assets => run_assets(&mut emitter),
        }
    } else {
        Err(ToolingError::new(ToolingErrorKind::UnsupportedProtocol))
    };
    let failure = outcome.err();
    let report = EndReport {
        status: if failure.is_none() {
            Outcome::Ok
        } else {
            Outcome::Failed
        },
        error: failure.map(|error| error.kind().as_str().to_owned()),
    };
    emitter.emit(Body::End(report))?;
    emitter
        .sink
        .flush()
        .map_err(|_| ToolingError::new(ToolingErrorKind::OutputFailed))?;
    failure.map_or(Ok(()), Err)
}

fn sorted_names(registry: &LiveRegistry) -> Result<Vec<ComponentName>, ToolingError> {
    let names = registry.component_names();
    if names.len() > MAX_COMPONENTS {
        return Err(ToolingError::new(ToolingErrorKind::ComponentLimitExceeded));
    }
    Ok(names)
}

fn run_check(emitter: &mut Emitter<'_>, roots: &[PathBuf]) -> Result<(), ToolingError> {
    let registry = App::resolve::<LiveRegistry>()
        .map_err(|_| ToolingError::new(ToolingErrorKind::RegistryUnavailable))?;
    let templates = load_templates(roots)?;
    let template_files = u32::try_from(templates.len())
        .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateLimitExceeded))?;
    let catalog = TemplateCatalog::new(templates)
        .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRejected))?;
    let names = sorted_names(&registry)?;
    let checker = TemplateChecker::new(registry.engine(), &catalog, CheckerLimits::default());
    let mut summary = CheckSummary {
        registry_bound: true,
        components: u32::try_from(names.len())
            .map_err(|_| ToolingError::new(ToolingErrorKind::ComponentLimitExceeded))?,
        proved: 0,
        errors: 0,
        unproved: 0,
        template_files,
    };
    let mut emitted = 0usize;
    for name in &names {
        let report = checker.check_component(name);
        if report.is_proved() {
            summary.proved += 1;
        }
        for diagnostic in report.diagnostics() {
            emitted += 1;
            if emitted > MAX_DIAGNOSTICS {
                return Err(ToolingError::new(ToolingErrorKind::DiagnosticLimitExceeded));
            }
            let severity = match diagnostic.severity() {
                DiagnosticSeverity::Error => {
                    summary.errors += 1;
                    Severity::Error
                }
                DiagnosticSeverity::Unproved => {
                    summary.unproved += 1;
                    Severity::Unproved
                }
            };
            emitter.emit(Body::Diagnostic(DiagnosticReport {
                component: diagnostic
                    .component()
                    .map(|component| component.as_str().to_owned()),
                view: diagnostic.path().map(|view| view.as_str().to_owned()),
                code: diagnostic.code().as_str().to_owned(),
                severity,
                line: diagnostic.line(),
                column: diagnostic.column(),
            }))?;
        }
    }
    emitter.emit(Body::Summary(summary))
}

struct TemplateBudget {
    files: usize,
    bytes: usize,
}

fn load_templates(roots: &[PathBuf]) -> Result<Vec<(ViewName, String)>, ToolingError> {
    if roots.len() > MAX_TEMPLATE_ROOTS {
        return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
    }
    let mut budget = TemplateBudget { files: 0, bytes: 0 };
    let mut seen = BTreeSet::new();
    let mut templates = Vec::new();
    for root in roots {
        let metadata = fs::symlink_metadata(root)
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRootRejected))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ToolingError::new(ToolingErrorKind::TemplateRootRejected));
        }
        walk_templates(root, root, 0, &mut budget, &mut seen, &mut templates)?;
    }
    Ok(templates)
}

fn walk_templates(
    root: &Path,
    directory: &Path,
    depth: usize,
    budget: &mut TemplateBudget,
    seen: &mut BTreeSet<ViewName>,
    templates: &mut Vec<(ViewName, String)>,
) -> Result<(), ToolingError> {
    if depth > MAX_TEMPLATE_DEPTH {
        return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRootRejected))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRootRejected))?;
    entries.sort();
    for path in entries {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRejected))?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(ToolingError::new(ToolingErrorKind::TemplateRejected));
        }
        if file_type.is_dir() {
            walk_templates(root, &path, depth + 1, budget, seen, templates)?;
            continue;
        }
        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("html") {
            continue;
        }
        budget.files += 1;
        if budget.files > MAX_TEMPLATE_FILES {
            return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
        }
        let declared = usize::try_from(metadata.len())
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateLimitExceeded))?;
        if declared > MAX_TEMPLATE_FILE_BYTES {
            return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRejected))?;
        let segments = relative
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .ok_or(ToolingError::new(ToolingErrorKind::TemplateRejected))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let view = ViewName::parse(&segments.join("/"))
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRejected))?;
        if !seen.insert(view.clone()) {
            return Err(ToolingError::new(ToolingErrorKind::TemplateRejected));
        }
        let source = fs::read_to_string(&path)
            .map_err(|_| ToolingError::new(ToolingErrorKind::TemplateRejected))?;
        if source.len() > MAX_TEMPLATE_FILE_BYTES {
            return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
        }
        budget.bytes += source.len();
        if budget.bytes > MAX_TEMPLATE_TOTAL_BYTES {
            return Err(ToolingError::new(ToolingErrorKind::TemplateLimitExceeded));
        }
        templates.push((view, source));
    }
    Ok(())
}

fn run_inspect(emitter: &mut Emitter<'_>) -> Result<(), ToolingError> {
    let registry = App::resolve::<LiveRegistry>().ok();
    let config = App::resolve::<LiveConfig>().unwrap_or_default();
    let upload_host = App::resolve::<LiveUploadHost>().ok();
    let runtime = LiveRuntime::bind().ok();
    let readiness = runtime.as_ref().map(|runtime| {
        let ready = runtime.readiness();
        ReadinessReport {
            clock: ready.clock,
            random: ready.random,
            key_ring: ready.key_ring,
            ledger: ready.ledger,
            promotion: ready.promotion,
            execution: ready.execution,
            context_validator: ready.context_validator,
            host_ports: ready.host_ports,
            upload_ports: ready.upload_ports,
            upload_services: ready.upload_services,
            mount_catalog: ready.mount_catalog,
            response_and_cancellation: ready.response_and_cancellation,
            subscription_ports: ready.subscription_ports,
            async_state: ready.async_state,
        }
    });
    let names = match &registry {
        Some(registry) => sorted_names(registry)?,
        None => Vec::new(),
    };
    emitter.emit(Body::Runtime(RuntimeReport {
        registry_bound: registry.is_some(),
        components: u32::try_from(names.len())
            .map_err(|_| ToolingError::new(ToolingErrorKind::ComponentLimitExceeded))?,
        config: ConfigReport {
            max_request_bytes: config.max_request_bytes() as u64,
            max_response_bytes: config.max_response_bytes() as u64,
            max_context_lifetime_ms: config.max_context_lifetime_ms(),
        },
        upload_host: UploadHostReport {
            installed: upload_host.is_some(),
            finalizer: upload_host
                .as_ref()
                .is_some_and(|host| host.finalizer().is_some()),
            direct_provider: upload_host
                .as_ref()
                .is_some_and(|host| host.direct_provider().is_some()),
            scanner: upload_host
                .as_ref()
                .is_some_and(|host| host.scanner().is_some()),
            application_validator: upload_host
                .as_ref()
                .is_some_and(|host| host.application_validator().is_some()),
        },
        runtime_bound: runtime.is_some(),
        readiness,
        asset_identity: emitter.assets.clone(),
        browser_runtime_version: BROWSER_RUNTIME_VERSION.to_owned(),
        runtime_contract_version: RUNTIME_CONTRACT_VERSION,
        protocol_versions: SUPPORTED_PROTOCOL_VERSIONS.to_vec(),
    }))?;
    if let Some(registry) = &registry {
        for name in &names {
            let descriptor = registry
                .engine()
                .resolve(name)
                .map_err(|_| ToolingError::new(ToolingErrorKind::RegistryUnavailable))?;
            emitter.emit(Body::Component(component_report(descriptor.metadata())))?;
        }
    }
    Ok(())
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn component_report(metadata: &ComponentMetadata) -> ComponentReport {
    let versions = metadata.versions();
    let digest: String = metadata.contract_digest().as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ComponentReport {
        name: metadata.identity().as_str().to_owned(),
        view: metadata.view().as_str().to_owned(),
        component_version: versions.component(),
        state_schema_version: versions.state_schema(),
        action_schema_version: versions.action_schema(),
        checker_contract_version: versions.checker_contract(),
        minimum_protocol: versions.minimum_protocol(),
        fields: count(metadata.fields().len()),
        upload_fields: count(
            metadata
                .fields()
                .iter()
                .filter(|field| field.upload_policy().is_some())
                .count(),
        ),
        actions: count(metadata.actions().len()),
        events: count(metadata.events().len()),
        effects: count(metadata.effects().len()),
        subscriptions: count(metadata.subscriptions().len()),
        refresh_on_promote: metadata.refresh_on_promote(),
        contract_digest: digest,
    }
}

fn asset_report(kind: AssetKind, file: &str, content_type: &str, bytes: &[u8]) -> AssetReport {
    let digest = Sha256::digest(bytes);
    AssetReport {
        kind,
        file: file.to_owned(),
        bytes: bytes.len() as u64,
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        sri: format!("sha256-{}", BASE64.encode(digest)),
        content_type: content_type.to_owned(),
        content: BASE64.encode(bytes),
    }
}

fn run_assets(emitter: &mut Emitter<'_>) -> Result<(), ToolingError> {
    let catalog =
        live_asset_catalog().map_err(|_| ToolingError::new(ToolingErrorKind::AssetsUnavailable))?;
    let mut reports = Vec::with_capacity(MAX_ASSETS);
    reports.push(asset_report(
        AssetKind::Manifest,
        MANIFEST_FILE,
        MANIFEST_CONTENT_TYPE,
        catalog.manifest_bytes(),
    ));
    for artifact in catalog.artifacts() {
        reports.push(asset_report(
            AssetKind::Artifact,
            artifact.file(),
            artifact.content_type(),
            artifact.bytes(),
        ));
    }
    for boot in catalog.boot_scripts() {
        reports.push(asset_report(
            AssetKind::Boot,
            boot.file(),
            ARTIFACT_CONTENT_TYPE,
            boot.bytes(),
        ));
    }
    let total: u64 = reports.iter().map(|report| report.bytes).sum();
    if reports.len() > MAX_ASSETS || total > MAX_ASSET_BYTES as u64 {
        return Err(ToolingError::new(ToolingErrorKind::OutputLimitExceeded));
    }
    for report in reports {
        emitter.emit(Body::Asset(report))?;
    }
    Ok(())
}

fn build_command() -> clap::Command {
    clap::Command::new(COMMAND_NAME)
        .hide(true)
        .about("Answers the suprnova CLI's Live tooling protocol (not for interactive use)")
        .arg(
            clap::Arg::new("protocol")
                .long("protocol")
                .required(true)
                .value_parser(clap::value_parser!(u16)),
        )
        .arg(clap::Arg::new("operation").long("operation").required(true))
        .arg(
            clap::Arg::new("templates")
                .long("templates")
                .action(clap::ArgAction::Append)
                .value_parser(clap::value_parser!(PathBuf)),
        )
}

fn run_command(
    matches: &clap::ArgMatches,
) -> Pin<Box<dyn Future<Output = Result<(), FrameworkError>> + Send>> {
    let protocol = matches.get_one::<u16>("protocol").copied().unwrap_or(0);
    let operation = matches
        .get_one::<String>("operation")
        .cloned()
        .unwrap_or_default();
    let roots: Vec<PathBuf> = matches
        .get_many::<PathBuf>("templates")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    Box::pin(async move {
        let request = ToolRequest::parse(protocol, &operation, roots)?;
        let stdout = std::io::stdout();
        let mut sink = stdout.lock();
        execute(&request, &mut sink)?;
        Ok(())
    })
}

inventory::submit! {
    CommandEntry {
        name: COMMAND_NAME,
        description: "Live tooling helper for the suprnova CLI",
        clap_builder: build_command,
        handler: run_command,
    }
}
