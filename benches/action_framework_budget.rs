//! A8/16 action-framework benchmark with external application work excluded.

#[path = "../tests/component_support.rs"]
mod component_support;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use bytes::Bytes;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use suprnova_live::action::{
    ActionArgumentSchema, ActionEntry, ActionError, ActionFuture, ActionResult, ActionTable,
    ActionTarget, AuthorizationRequirement, AuthorizedAction, PreparedActionArguments,
    RawActionArguments, TransactionPolicy,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{
    ComponentError, ComponentFactory, ComponentHooks, ComponentInstance, HydrationContext,
    LiveFuture, MountContext, RenderContext,
};
use suprnova_live::execution::ExecutionResult;
use suprnova_live::identity::{
    ActionName, BuildId, ComponentName, IslandSlot, ModelField, Revision, UnixMillis, ViewName,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::protocol::{
    ProtocolLimitConfig, ProtocolLimits, VersionedUpdateRequest, parse_versioned_update_request,
};
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema,
};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, SnapshotLimits, SnapshotSchemaSet,
};
use suprnova_live::state::{
    BindingTiming, ModelBindingSchema, ModelCodec, ModelFieldBinding, ProposalBatch,
    ProposalLimits, RawModelProposal,
};
use suprnova_live::validation::ValidationSelection;
use suprnova_live::view::{AssetSet, IslandRender};
use suprnova_live_test_support::{
    ComponentHarness, ComponentHarnessConfig, HarnessRequestIdentity, HarnessServices,
};

const STATE_BYTES: usize = 8 * 1024;
const HTML_BYTES: usize = 16 * 1024;
const WARMUP_ITERATIONS: usize = 50;
const MEASURED_SAMPLES: usize = 40;
const P95_CAP_MICROSECONDS: f64 = 2_000.0;
const NOW: UnixMillis = UnixMillis::new(1_000);

struct BudgetFactory;

struct BudgetInstance {
    state: CanonicalValue,
}

impl ComponentInstance for BudgetInstance {
    fn metadata(&self) -> &'static ComponentMetadata {
        metadata()
    }

    fn bind_models(&mut self, proposals: &ProposalBatch) -> Result<(), ComponentError> {
        if proposals.issues().is_empty() {
            Ok(())
        } else {
            Err(ComponentError::contract_failure())
        }
    }

    fn render<'a>(
        &'a self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        Box::pin(async {
            let mut body = String::with_capacity(HTML_BYTES);
            body.push_str("<div>");
            body.push_str(&"h".repeat(HTML_BYTES - "<div></div>".len()));
            body.push_str("</div>");
            Ok(IslandRender {
                body: Bytes::from(body),
                assets: AssetSet::empty(),
                children: vec![],
            })
        })
    }

    fn dehydrate(&self, _exposure: StateExposure) -> Result<CanonicalValue, ComponentError> {
        Ok(self.state.clone())
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(BTreeMap::new()))
    }
}

impl ComponentFactory for BudgetFactory {
    fn mount<'a>(
        &'a self,
        _context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async {
            Ok(Box::new(BudgetInstance {
                state: benchmark_state(),
            }) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            Ok(Box::new(BudgetInstance {
                state: context.state().clone(),
            }) as Box<dyn ComponentInstance>)
        })
    }
}

fn dispatch_no_application_work<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        target
            .as_any_mut()
            .downcast_mut::<BudgetInstance>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        Ok(ActionResult::no_render())
    })
}

fn metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            ComponentName::parse("bench.action-framework").expect("component identity"),
            ViewName::parse("bench/action-framework.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("contract versions"),
            vec![
                FieldMetadata::new(
                    ModelField::parse("payload").expect("payload field"),
                    FieldCategory::State,
                    StateCodec::Json,
                    true,
                ),
                FieldMetadata::new(
                    ModelField::parse("query").expect("query field"),
                    FieldCategory::Model,
                    StateCodec::Json,
                    true,
                )
                .with_model_binding(ModelCodec::String, BindingTiming::Submit)
                .expect("query model metadata"),
            ],
            vec![
                ActionMetadata::new_with_contract(
                    ActionName::parse("advance").expect("action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Current,
                    ValidationSelection::None,
                    TransactionPolicy::None,
                )
                .expect("action metadata"),
            ],
        )
        .expect("component metadata")
    })
}

fn schemas() -> SnapshotSchemaSet {
    SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("payload", StateCodec::Json, FieldCategory::State, true)
                    .expect("payload state field"),
                FieldSpec::new("query", StateCodec::Json, FieldCategory::Model, true)
                    .expect("query state field"),
            ],
        )
        .expect("state schema"),
        StateSchema::new(1, vec![]).expect("memo schema"),
        StateSchema::new(1, vec![]).expect("mount schema"),
    )
    .expect("snapshot schemas")
}

fn benchmark_state() -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        (
            "payload".to_owned(),
            CanonicalValue::String("s".repeat(STATE_BYTES)),
        ),
        (
            "query".to_owned(),
            CanonicalValue::String("rust".to_owned()),
        ),
    ]))
}

fn protocol_limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("protocol limits")
}

fn snapshot_limits() -> SnapshotLimits {
    SnapshotLimits::new(
        InputLimits::new(64 * 1024, 12, 512, 32 * 1024).expect("snapshot input limits"),
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .expect("snapshot limits")
}

struct Fixture {
    harness: ComponentHarness,
    action: ActionName,
    proposals: ProposalBatch,
    limits: ProtocolLimits,
    initial_fixture_digest: String,
}

impl Fixture {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let services = HarnessServices::new(NOW);
        let schemas = schemas();
        let actions = ActionTable::new(vec![ActionEntry::new(
            metadata().actions()[0].clone(),
            dispatch_no_application_work,
        )])?;
        let descriptor = ComponentDescriptor::with_hooks(
            metadata().clone(),
            ComponentHooks::new(Arc::new(BudgetFactory)),
        )
        .with_actions(actions)?;
        let context = component_support::trusted_context_for_with_schemas(
            metadata(),
            Some(Arc::clone(services.authorization())
                as Arc<dyn suprnova_live::action::ActionAuthorizationPort>),
            schemas.clone(),
        );
        let expected = ExpectedInstanceV1::new(
            ComponentContract::new(
                metadata().identity().clone(),
                descriptor.contract_digest().clone(),
                1,
                1,
                1,
            )?,
            BuildId::parse("build-lifecycle-tests")?,
            component_support::snapshot_support::route(0x30),
            IslandSlot::parse("trace")?,
            context.scope().clone(),
            schemas,
        );
        let config = ComponentHarnessConfig::new(
            descriptor,
            context,
            expected,
            component_support::key_ring(),
            snapshot_limits(),
            services,
        );
        let mut harness = ComponentHarness::new(config)?;
        let mounted = match harness.mount(CanonicalValue::Object(BTreeMap::new())).await {
            Ok(mounted) => mounted,
            Err(error) => {
                eprintln!("mount trace: {:?}", harness.services().trace().events());
                return Err(error.into());
            }
        };
        if mounted.body().len() <= HTML_BYTES {
            return Err(std::io::Error::other("mount wrapper did not contain A8/16 HTML").into());
        }
        let proposal_schema = ModelBindingSchema::new(vec![ModelFieldBinding::new(
            "query",
            FieldCategory::Model,
            ModelCodec::String,
        )?])?;
        let proposals = ProposalBatch::prepare(
            &proposal_schema,
            vec![RawModelProposal::new(
                "query",
                CanonicalValue::String("live".to_owned()),
            )],
            &ProposalLimits::default(),
        )?;
        let initial_fixture_digest = sha256_hex(
            harness
                .current_encoded_snapshot()
                .ok_or_else(|| std::io::Error::other("mounted snapshot is absent"))?,
        );
        Ok(Self {
            harness,
            action: ActionName::parse("advance")?,
            proposals,
            limits: protocol_limits(),
            initial_fixture_digest,
        })
    }

    fn request_bytes(&self, seed: u8) -> Result<Vec<u8>, Box<dyn Error>> {
        let envelope: serde_json::Value = serde_json::from_slice(
            self.harness
                .current_encoded_snapshot()
                .ok_or_else(|| std::io::Error::other("current snapshot is absent"))?,
        )?;
        let revision = self
            .harness
            .current_snapshot()
            .ok_or_else(|| std::io::Error::other("current state is absent"))?
            .body()
            .revision();
        let idempotency = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([seed; 16]);
        let correlation =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([seed.wrapping_add(1); 16]);
        Ok(serde_json_canonicalizer::to_vec(&serde_json::json!({
            "base_revision": revision.get().to_string(),
            "child_parameters": null,
            "component": metadata().identity().as_str(),
            "correlation_id": correlation,
            "extensions": {},
            "idempotency_key": idempotency,
            "model_proposals": {"query": "live"},
            "operations": [{"arguments": {}, "kind": "invoke_action", "name": "advance"}],
            "protocol_version": 2,
            "runtime_contract_version": 2,
            "snapshot": {"envelope": envelope, "kind": "instance"},
            "snapshot_schema_version": 1
        }))?)
    }

    async fn process_once(&mut self, seed: u8) -> Result<Revision, Box<dyn Error>> {
        let request_bytes = self.request_bytes(seed)?;
        let parsed = parse_versioned_update_request(&request_bytes, &self.limits)?;
        let VersionedUpdateRequest::V2(parsed) = parsed else {
            return Err(std::io::Error::other("A8/16 request did not resolve as v2").into());
        };
        black_box(parsed);
        let result = self
            .harness
            .execute_action(
                &self.action,
                RawActionArguments::empty(),
                Some(&self.proposals),
                HarnessRequestIdentity::from_seed(seed),
            )
            .await?;
        let ExecutionResult::Accepted(accepted) = result else {
            return Err(std::io::Error::other("A8/16 action was not accepted").into());
        };
        if accepted.render().is_some() {
            return Err(
                std::io::Error::other("Askama render entered the timed action path").into(),
            );
        }
        Ok(black_box(accepted.revision()))
    }
}

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: u8,
    workload: &'static str,
    state_bytes: usize,
    html_bytes: usize,
    warmup_iterations: usize,
    measured_samples: usize,
    p50_microseconds: f64,
    p95_microseconds: f64,
    stages: [&'static str; 7],
    excluded: [&'static str; 3],
    profile: &'static str,
    fixture_sha256: String,
    measured_at_unix_ms: u128,
    environment: EnvironmentEvidence,
}

#[derive(Serialize)]
struct EnvironmentEvidence {
    classification: &'static str,
    operating_system: &'static str,
    architecture: &'static str,
    cpu_model: String,
    selected_cpu_affinity: String,
    selected_cpu_count: usize,
    memory_bytes: u64,
    kernel: String,
    cpu_governor: String,
    rustc: String,
    database: &'static str,
    provider_versions: BTreeMap<&'static str, &'static str>,
    dedicated_vcpus_attested: bool,
    warm_filesystem_cache: bool,
    loopback_providers: bool,
    s1_requirements_met: bool,
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
        let governor = governors(&affinity);
        let dedicated = std::env::var("SUPRNOVA_LIVE_S1_DEDICATED").as_deref() == Ok("1");
        let requirements_met = std::env::consts::OS == "linux"
            && std::env::consts::ARCH == "x86_64"
            && selected_cpu_count == 8
            && memory_bytes >= 16 * 1024 * 1024 * 1024
            && governor == "performance"
            && dedicated;
        Self {
            classification: if requirements_met {
                "validated_s1"
            } else {
                "local_exploratory"
            },
            operating_system: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            cpu_model: read_cpu_model().unwrap_or_else(|| "unavailable".to_owned()),
            selected_cpu_affinity: affinity,
            selected_cpu_count,
            memory_bytes,
            kernel: command_output("uname", &["-sr"]).unwrap_or_else(|| "unavailable".to_owned()),
            cpu_governor: governor,
            rustc: command_output("rustc", &["--version"])
                .unwrap_or_else(|| "unavailable".to_owned()),
            database: "not_used_by_action_framework_benchmark",
            provider_versions: BTreeMap::from([
                ("instance_ledger", "in_process_tier0_v1"),
                ("snapshot_key_ring", "in_process_v1"),
            ]),
            dedicated_vcpus_attested: dedicated,
            warm_filesystem_cache: true,
            loopback_providers: true,
            s1_requirements_met: requirements_met,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("action framework budget failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new().await?;
    if cfg!(debug_assertions) {
        fixture.process_once(1).await?;
        println!("action framework budget debug contract check only; release timing skipped");
        return Ok(());
    }

    for iteration in 0..WARMUP_ITERATIONS {
        black_box(fixture.process_once(iteration as u8 + 1).await?);
    }
    let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
    for iteration in 0..MEASURED_SAMPLES {
        let seed = (WARMUP_ITERATIONS + iteration + 1) as u8;
        let started = Instant::now();
        black_box(fixture.process_once(seed).await?);
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    samples.sort_by(f64::total_cmp);
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let environment = EnvironmentEvidence::collect();
    let result = BenchmarkResult {
        schema_version: 1,
        workload: "A8/16-action-framework",
        state_bytes: STATE_BYTES,
        html_bytes: HTML_BYTES,
        warmup_iterations: WARMUP_ITERATIONS,
        measured_samples: MEASURED_SAMPLES,
        p50_microseconds: p50,
        p95_microseconds: p95,
        stages: [
            "parse",
            "verify",
            "claim",
            "hydrate",
            "bind",
            "dispatch",
            "successor_classify",
        ],
        excluded: ["application_action_body", "provider_io", "askama_render"],
        profile: "release",
        fixture_sha256: fixture.initial_fixture_digest,
        measured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        environment,
    };
    write_result(&result, result_path())?;
    println!(
        "A8/16 action framework: p50={:.3}us p95={:.3}us environment={}",
        result.p50_microseconds, result.p95_microseconds, result.environment.classification
    );
    if p95 >= P95_CAP_MICROSECONDS {
        return Err(std::io::Error::other("action framework reached the 2 ms p95 ceiling").into());
    }
    if std::env::var_os("SUPRNOVA_LIVE_REQUIRE_S1").is_some()
        && !result.environment.s1_requirements_met
    {
        return Err(
            std::io::Error::other("runner cannot prove the required S1 environment").into(),
        );
    }
    Ok(())
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    let index = ((samples.len() - 1) as f64 * percentile).ceil() as usize;
    samples[index]
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_BENCH_RESULT").map_or_else(
        || PathBuf::from("benchmarks/action-budget-v1.json"),
        PathBuf::from,
    )
}

fn write_result(result: &BenchmarkResult, path: PathBuf) -> Result<(), Box<dyn Error>> {
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
