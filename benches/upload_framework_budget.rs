//! U4/16 upload control-framework benchmark with every external work port excluded.

#[path = "../tests/component_support.rs"]
mod component_support;

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde::Serialize;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, ModelField, UnixMillis};
use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
use suprnova_live::upload::{
    AcceptedChunk, ApplicationValidationDecision, ApplicationValidationInput, ChunkBody,
    IntegrityEvidence, PrepareTransfer, QuarantineBytes, ReadUpload, ScanDisposition, ScanInput,
    TransferGrant, TransferGrantCodec, TransferGrantRequest, TransferGrantScope, TransferPlan,
    TransitionDisposition, UploadApplicationValidator, UploadError, UploadErrorKind, UploadFuture,
    UploadHandle, UploadLedger, UploadOperation, UploadProtocolCodec, UploadProvider, UploadRecord,
    UploadRevision, UploadScanner, UploadService, UploadState, UploadTransition,
    UploadTransitionAdmission, UploadTransitionRequest, VerifyTransfer,
};
use suprnova_live_test_support::{ControlledUploadAuthorization, MemoryUploadLedger};

const WORKLOAD: &str = "U4/16";
const FILES: usize = 4;
const FILE_BYTES: usize = 16 * 1024 * 1024;
const CHUNK_BYTES: usize = 256 * 1024;
const ACTIVE_TRANSFERS: usize = 4;
const WARMUP_ITERATIONS: usize = 50;
const MEASURED_SAMPLES: usize = 40;
const P95_CAP_MICROSECONDS: f64 = 2_000.0;
const NOW: UnixMillis = UnixMillis::new(1_001);
const EXPIRES_AT: UnixMillis = UnixMillis::new(1_900);
const ROOT_SECRET: &[u8] = b"upload-budget-root-secret-000000";

#[derive(Default)]
struct ExcludedPortCounters {
    body_io: AtomicUsize,
    provider: AtomicUsize,
    scanner: AtomicUsize,
    application_validation: AtomicUsize,
}

struct NullChunkBody {
    counters: Arc<ExcludedPortCounters>,
}

impl ChunkBody for NullChunkBody {
    fn next_chunk<'a>(
        &'a mut self,
        _maximum_bytes: usize,
    ) -> UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
        self.counters.body_io.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullProvider {
    counters: Arc<ExcludedPortCounters>,
}

impl UploadProvider for NullProvider {
    fn prepare<'a>(
        &'a self,
        _request: PrepareTransfer<'a>,
    ) -> UploadFuture<'a, Result<TransferPlan, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn verify<'a>(
        &'a self,
        _request: VerifyTransfer<'a>,
    ) -> UploadFuture<'a, Result<IntegrityEvidence, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn read<'a>(
        &'a self,
        _request: ReadUpload<'a>,
    ) -> UploadFuture<'a, Result<QuarantineBytes, UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn cancel<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }

    fn cleanup<'a>(
        &'a self,
        _handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        self.counters.provider.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullScanner {
    counters: Arc<ExcludedPortCounters>,
}

impl UploadScanner for NullScanner {
    fn scan<'a>(
        &'a self,
        _input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>> {
        self.counters.scanner.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullApplicationValidator {
    counters: Arc<ExcludedPortCounters>,
}

impl UploadApplicationValidator for NullApplicationValidator {
    fn validate<'a>(
        &'a self,
        _input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        self.counters
            .application_validation
            .fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(UploadError::new(UploadErrorKind::ProviderUnavailable)) })
    }
}

struct NullExternalPorts {
    _application: Arc<dyn UploadApplicationValidator>,
    _body: NullChunkBody,
    _provider: Arc<dyn UploadProvider>,
    _scanner: Arc<dyn UploadScanner>,
    counters: Arc<ExcludedPortCounters>,
}

impl NullExternalPorts {
    fn new() -> Self {
        let counters = Arc::new(ExcludedPortCounters::default());
        Self {
            _application: Arc::new(NullApplicationValidator {
                counters: Arc::clone(&counters),
            }),
            _body: NullChunkBody {
                counters: Arc::clone(&counters),
            },
            _provider: Arc::new(NullProvider {
                counters: Arc::clone(&counters),
            }),
            _scanner: Arc::new(NullScanner {
                counters: Arc::clone(&counters),
            }),
            counters,
        }
    }

    fn require_zero(&self) -> Result<(), Box<dyn Error>> {
        self.counters.require_zero()
    }
}

impl ExcludedPortCounters {
    fn snapshot(&self) -> ExcludedCalls {
        ExcludedCalls {
            application_validation: self.application_validation.load(Ordering::SeqCst),
            body_io: self.body_io.load(Ordering::SeqCst),
            provider: self.provider.load(Ordering::SeqCst),
            scanner: self.scanner.load(Ordering::SeqCst),
        }
    }

    fn require_zero(&self) -> Result<(), Box<dyn Error>> {
        let calls = self.snapshot();
        if calls != ExcludedCalls::default() {
            return Err(
                std::io::Error::other("external upload work entered timed control path").into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
struct ExcludedCalls {
    #[serde(rename = "applicationValidation")]
    application_validation: usize,
    #[serde(rename = "bodyIo")]
    body_io: usize,
    provider: usize,
    scanner: usize,
}

struct Fixture {
    service: UploadService,
    context: suprnova_live::host::TrustedLiveRequestContext,
    grant: TransferGrant,
    request: Vec<u8>,
    codec: UploadProtocolCodec,
    authorization: Arc<ControlledUploadAuthorization>,
    excluded: NullExternalPorts,
}

impl Fixture {
    fn new(index: usize) -> Result<Self, Box<dyn Error>> {
        let limits = UploadLimits::new(UploadLimitConfig::reference())?;
        let authorization = Arc::new(ControlledUploadAuthorization::new());
        let context = component_support::trusted_context_with_upload_authorization(Arc::clone(
            &authorization,
        )
            as Arc<dyn suprnova_live::upload::UploadAuthorizationPort>);
        let handle = UploadHandle::parse(&format!("018f47c1-2af0-7cc4-a001-{index:012}"))?;
        let field = ModelField::parse("serial")?;
        let authority = TransferGrantScope::new(
            handle.clone(),
            context.mount().component().clone(),
            field,
            component_support::fixture_host_scope(),
            1,
        );
        let codec = grant_codec()?;
        let issued = codec.issue(
            TransferGrantRequest::new(authority.clone(), EXPIRES_AT),
            UnixMillis::new(1_000),
        )?;
        let grant = TransferGrant::parse(issued.grant().expose_bearer())?;
        let ledger = Arc::new(MemoryUploadLedger::new(limits)?);
        ledger.seed(UploadRecord::new(
            authority,
            UploadState::Transferring,
            UploadRevision::new(7),
            UnixMillis::new(1_000),
            EXPIRES_AT,
        )?)?;
        let service_ledger: Arc<dyn UploadLedger> = ledger;
        let service = UploadService::new(service_ledger, codec, limits)?;
        let request = serde_json_canonicalizer::to_vec(&serde_json::json!({
            "checksum": "ab".repeat(32),
            "chunk_index": 0,
            "expected_revision": "7",
            "handle": handle.to_string(),
            "idempotency_key": format!("u4-16-{index}"),
            "operation": "put_chunk",
            "protocol_version": 1,
            "size": CHUNK_BYTES,
        }))?;
        Ok(Self {
            service,
            context,
            grant,
            request,
            codec: UploadProtocolCodec::v1(),
            authorization,
            excluded: NullExternalPorts::new(),
        })
    }

    async fn process_once(&self) -> Result<(), Box<dyn Error>> {
        let operation = self.codec.decode(&self.request)?;
        let UploadOperation::PutChunk(chunk) = operation else {
            return Err(std::io::Error::other("U4/16 control did not decode as put_chunk").into());
        };
        let transition = UploadTransitionRequest::new(
            chunk.handle().clone(),
            chunk.expected_revision(),
            chunk.idempotency_key().clone(),
            UploadTransition::PutChunk(AcceptedChunk::new(
                chunk.chunk_index(),
                chunk.size(),
                chunk.checksum().clone(),
            )?),
        );
        let admission = UploadTransitionAdmission::new(
            self.grant.clone(),
            ModelField::parse("serial")?,
            transition,
        );
        let applied = self
            .service
            .transition(&self.context, admission.clone(), NOW)
            .await?;
        let applied_response = response_bytes(applied)?;
        let duplicate = self
            .service
            .transition(&self.context, admission, NOW)
            .await?;
        let duplicate_response = response_bytes(duplicate)?;
        if applied.disposition() != TransitionDisposition::Applied
            || duplicate.disposition() != TransitionDisposition::ExistingOutcome
            || applied.state() != UploadState::Transferring
            || duplicate.state() != applied.state()
            || duplicate.revision() != applied.revision()
            || applied_response.is_empty()
            || duplicate_response.is_empty()
            || self.authorization.call_count() != 2
        {
            return Err(std::io::Error::other("U4/16 control/idempotency contract failed").into());
        }
        self.excluded.require_zero()?;
        black_box((applied_response, duplicate_response));
        Ok(())
    }
}

fn response_bytes(
    outcome: suprnova_live::upload::TransitionOutcome,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json_canonicalizer::to_vec(&serde_json::json!({
        "disposition": match outcome.disposition() {
            TransitionDisposition::Applied => "applied",
            TransitionDisposition::ExistingOutcome => "existing_outcome",
        },
        "revision": outcome.revision().get().to_string(),
        "state": outcome.state().as_str(),
    }))
}

fn grant_codec() -> Result<TransferGrantCodec, Box<dyn Error>> {
    let key = KeyRecord::new(
        KeyId::parse("upload-budget-key")?,
        RootKey::new(ROOT_SECRET.to_vec())?,
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )?;
    Ok(TransferGrantCodec::new(SnapshotKeyRing::new(
        key,
        Vec::new(),
    )?))
}

#[derive(Serialize)]
struct ServerBudgetResult {
    bounds: ServerBounds,
    environment: EnvironmentEvidence,
    measurements: ServerMeasurements,
    methodology: Methodology,
    workload: Workload,
}

#[derive(Serialize)]
struct ServerBounds {
    #[serde(rename = "maxChunksPerActiveTransfer")]
    max_chunks_per_active_transfer: usize,
    #[serde(rename = "maxControlP95Microseconds")]
    max_control_p95_microseconds: usize,
    #[serde(rename = "maxManagerOwnedBytes")]
    max_manager_owned_bytes: usize,
}

#[derive(Serialize)]
struct ServerMeasurements {
    #[serde(rename = "excludedCalls")]
    excluded_calls: ExcludedCalls,
    #[serde(rename = "liveChunkBuffers")]
    live_chunk_buffers: usize,
    #[serde(rename = "managerOwnedBytes")]
    manager_owned_bytes: usize,
    #[serde(rename = "maxChunksPerTransfer")]
    max_chunks_per_transfer: usize,
    #[serde(rename = "maxConcurrentTransfers")]
    max_concurrent_transfers: usize,
    #[serde(rename = "maxQueueDepth")]
    max_queue_depth: usize,
    #[serde(rename = "p50Microseconds")]
    p50_microseconds: f64,
    #[serde(rename = "p95Microseconds")]
    p95_microseconds: f64,
    #[serde(rename = "retainedBytes")]
    retained_bytes: usize,
}

#[derive(Serialize)]
struct Methodology {
    #[serde(rename = "measuredSamples")]
    measured_samples: usize,
    #[serde(rename = "warmupIterations")]
    warmup_iterations: usize,
}

#[derive(Clone, Copy, Serialize)]
struct Workload {
    #[serde(rename = "activeTransfers")]
    active_transfers: usize,
    #[serde(rename = "chunkBytes")]
    chunk_bytes: usize,
    #[serde(rename = "fileBytes")]
    file_bytes: usize,
    files: usize,
}

#[derive(Serialize)]
struct EnvironmentEvidence {
    architecture: &'static str,
    classification: &'static str,
    #[serde(rename = "cpuGovernor")]
    cpu_governor: String,
    #[serde(rename = "cpuModel")]
    cpu_model: String,
    database: &'static str,
    #[serde(rename = "dedicatedVcpusAttested")]
    dedicated_vcpus_attested: bool,
    kernel: String,
    #[serde(rename = "loopbackProviders")]
    loopback_providers: bool,
    #[serde(rename = "memoryBytes")]
    memory_bytes: u64,
    #[serde(rename = "operatingSystem")]
    operating_system: &'static str,
    profile: &'static str,
    #[serde(rename = "qualificationRequirementsMet")]
    qualification_requirements_met: bool,
    rustc: String,
    #[serde(rename = "selectedCpuCount")]
    selected_cpu_count: usize,
    #[serde(rename = "warmFilesystemCache")]
    warm_filesystem_cache: bool,
}

impl EnvironmentEvidence {
    fn collect() -> Self {
        let affinity = read_labeled_value("/proc/self/status", "Cpus_allowed_list")
            .unwrap_or_else(|| "unavailable".to_owned());
        let selected_cpu_count = cpu_list(&affinity).len();
        let memory_bytes = read_labeled_value("/proc/meminfo", "MemTotal")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
            .saturating_mul(1024);
        let cpu_governor = governors(&affinity);
        let dedicated = std::env::var("SUPRNOVA_LIVE_S1_DEDICATED").as_deref() == Ok("1");
        let requirements_met = std::env::consts::OS == "linux"
            && std::env::consts::ARCH == "x86_64"
            && selected_cpu_count == 8
            && memory_bytes >= 16 * 1024 * 1024 * 1024
            && cpu_governor == "performance"
            && dedicated;
        Self {
            architecture: std::env::consts::ARCH,
            classification: if requirements_met {
                "qualified"
            } else {
                "unqualified"
            },
            cpu_governor,
            cpu_model: read_cpu_model().unwrap_or_else(|| "unavailable".to_owned()),
            database: "not_used_by_upload_control_budget",
            dedicated_vcpus_attested: dedicated,
            kernel: command_output("uname", &["-sr"]).unwrap_or_else(|| "unavailable".to_owned()),
            loopback_providers: true,
            memory_bytes,
            operating_system: std::env::consts::OS,
            profile: "S1",
            qualification_requirements_met: requirements_met,
            rustc: command_output("rustc", &["--version"])
                .unwrap_or_else(|| "unavailable".to_owned()),
            selected_cpu_count,
            warm_filesystem_cache: true,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("upload framework budget failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        println!("upload framework budget debug contract check only; release timing skipped");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<(), Box<dyn Error>> {
    let mut fixtures = Vec::with_capacity(WARMUP_ITERATIONS + MEASURED_SAMPLES);
    for index in 1..=WARMUP_ITERATIONS + MEASURED_SAMPLES {
        fixtures.push(Fixture::new(index)?);
    }
    for fixture in fixtures.iter().take(WARMUP_ITERATIONS) {
        black_box(fixture.process_once().await?);
    }
    let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
    for fixture in fixtures.iter().skip(WARMUP_ITERATIONS) {
        let started = Instant::now();
        black_box(fixture.process_once().await?);
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    samples.sort_by(f64::total_cmp);
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let environment = EnvironmentEvidence::collect();
    let result = ServerBudgetResult {
        bounds: ServerBounds {
            max_chunks_per_active_transfer: 2,
            max_control_p95_microseconds: P95_CAP_MICROSECONDS as usize,
            max_manager_owned_bytes: 512 * 1024,
        },
        environment,
        measurements: ServerMeasurements {
            excluded_calls: ExcludedCalls::default(),
            live_chunk_buffers: 0,
            manager_owned_bytes: 0,
            max_chunks_per_transfer: 0,
            max_concurrent_transfers: 0,
            max_queue_depth: 0,
            p50_microseconds: p50,
            p95_microseconds: p95,
            retained_bytes: 0,
        },
        methodology: Methodology {
            measured_samples: MEASURED_SAMPLES,
            warmup_iterations: WARMUP_ITERATIONS,
        },
        workload: Workload {
            active_transfers: ACTIVE_TRANSFERS,
            chunk_bytes: CHUNK_BYTES,
            file_bytes: FILE_BYTES,
            files: FILES,
        },
    };
    write_result(&result, result_path())?;
    println!(
        "{WORKLOAD} upload control: p50={:.3}us p95={:.3}us environment={}",
        p50, p95, result.environment.classification
    );
    if p95 > P95_CAP_MICROSECONDS {
        return Err(std::io::Error::other("upload control exceeded the 2 ms p95 ceiling").into());
    }
    if std::env::var_os("SUPRNOVA_LIVE_REQUIRE_S1").is_some()
        && !result.environment.qualification_requirements_met
    {
        return Err(
            std::io::Error::other("runner cannot prove the required S1 environment").into(),
        );
    }
    Ok(())
}

fn percentile(samples: &[f64], quantile: f64) -> f64 {
    let index = ((samples.len() as f64 * quantile).ceil() as usize).saturating_sub(1);
    samples[index]
}

fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_UPLOAD_SERVER_RESULT").map_or_else(
        || PathBuf::from("benchmarks/local/upload-server-v1.json"),
        PathBuf::from,
    )
}

fn write_result(result: &ServerBudgetResult, path: PathBuf) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(&path);
    fs::write(&temporary, serde_json::to_vec_pretty(result)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".tmp");
    PathBuf::from(value)
}

fn read_labeled_value(path: &str, label: &str) -> Option<String> {
    fs::read_to_string(path).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == label).then(|| value.trim().to_owned())
    })
}

fn read_cpu_model() -> Option<String> {
    read_labeled_value("/proc/cpuinfo", "model name")
}

fn cpu_list(value: &str) -> BTreeSet<u32> {
    let mut cpus = BTreeSet::new();
    for part in value.split(',') {
        let mut bounds = part.trim().splitn(2, '-');
        let Some(start) = bounds.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let end = bounds
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(start);
        cpus.extend(start..=end);
    }
    cpus
}

fn governors(affinity: &str) -> String {
    let values = cpu_list(affinity)
        .into_iter()
        .filter_map(|cpu| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .ok()
            .map(|value| value.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if values.is_empty() {
        "unavailable".to_owned()
    } else {
        values.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
