//! Release-only A8/16 trusted snapshot-processing budget executable.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use suprnova_live::canonical::to_canonical_bytes;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, InstanceId, IslandSlot, KeyId, Revision, RouteIdentity,
    ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema, dehydrate,
};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, InstanceBodyV1, InstanceFieldsV1, SnapshotLimits,
    SnapshotSchemaSet, verify_instance,
};

const STATE_BYTES: usize = 8 * 1024;
const HTML_BYTES: usize = 16 * 1024;
const WARMUP_ITERATIONS: usize = 500;
const MEASURED_SAMPLES: usize = 40;
const BATCH_ITERATIONS: usize = 100;
const P95_CAP_MICROSECONDS: f64 = 500.0;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BenchmarkState {
    payload: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct BenchmarkMemo {}

struct Fixture {
    encoded: Vec<u8>,
    fields: InstanceFieldsV1,
    expected: ExpectedInstanceV1,
    schemas: SnapshotSchemaSet,
    limits: SnapshotLimits,
    keys: SnapshotKeyRing,
}

struct PipelineOutput {
    signed: Vec<u8>,
    canonical_state_bytes: usize,
    hydrated_state: BenchmarkState,
}

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: u16,
    workload: &'static str,
    state_bytes: usize,
    html_bytes: usize,
    control_overhead_bytes: usize,
    snapshot_overhead_bytes: usize,
    warmup_iterations: usize,
    measured_samples: usize,
    batch_iterations: usize,
    p50_microseconds: f64,
    p95_microseconds: f64,
    stages: [&'static str; 5],
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

fn main() {
    if let Err(error) = run() {
        eprintln!("snapshot budget failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        Fixture::new()?.assert_correctness_guards()?;
        println!("snapshot budget debug contract check only; release timing skipped");
        return Ok(());
    }

    let fixture = Fixture::new()?;
    fixture.assert_correctness_guards()?;

    for _ in 0..WARMUP_ITERATIONS {
        black_box(fixture.process_once()?);
    }

    let mut samples = Vec::with_capacity(MEASURED_SAMPLES);
    for _ in 0..MEASURED_SAMPLES {
        let started = Instant::now();
        for _ in 0..BATCH_ITERATIONS {
            black_box(fixture.process_once()?);
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / BATCH_ITERATIONS as f64);
    }
    samples.sort_by(f64::total_cmp);

    let environment = EnvironmentEvidence::collect();
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let result = BenchmarkResult {
        schema_version: 1,
        workload: "A8/16",
        state_bytes: STATE_BYTES,
        html_bytes: HTML_BYTES,
        control_overhead_bytes: fixture.control_overhead_bytes()?,
        snapshot_overhead_bytes: fixture.snapshot_overhead_bytes()?,
        warmup_iterations: WARMUP_ITERATIONS,
        measured_samples: MEASURED_SAMPLES,
        batch_iterations: BATCH_ITERATIONS,
        p50_microseconds: p50,
        p95_microseconds: p95,
        stages: ["verify", "hydrate", "dehydrate", "canonicalize", "sign"],
        profile: "release",
        fixture_sha256: sha256_hex(&fixture.encoded),
        measured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        environment,
    };

    write_result(&result, result_path())?;
    println!(
        "A8/16 snapshot processing: p50={:.3}us p95={:.3}us environment={}",
        result.p50_microseconds, result.p95_microseconds, result.environment.classification
    );

    if result.control_overhead_bytes > 1_024 {
        return Err(std::io::Error::other("control overhead exceeds 1 KiB").into());
    }
    if result.snapshot_overhead_bytes > 768 {
        return Err(std::io::Error::other("snapshot overhead exceeds 768 bytes").into());
    }
    if p95 > P95_CAP_MICROSECONDS {
        return Err(
            std::io::Error::other("snapshot processing exceeds 500 microseconds p95").into(),
        );
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

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let input_limits = InputLimits::new(32 * 1024, 12, 512, 16 * 1024)?;
        let limits = SnapshotLimits::new(input_limits, 50, 10_000, 20_000, 8, 8)?;
        let keys = SnapshotKeyRing::new(
            KeyRecord::new(
                KeyId::parse("snapshot-budget-v1")?,
                RootKey::new(vec![0x42; 32])?,
                UnixMillis::new(0),
                UnixMillis::new(10_000),
                UnixMillis::new(20_000),
            )?,
            Vec::new(),
        )?;
        let state_schema = StateSchema::new(
            1,
            vec![FieldSpec::new(
                "payload",
                StateCodec::Json,
                FieldCategory::Public,
                true,
            )?],
        )?;
        let memo_schema = StateSchema::new(1, Vec::new())?;
        let mount_schema = StateSchema::new(1, Vec::new())?;
        let schemas = SnapshotSchemaSet::new(state_schema, memo_schema, mount_schema)?;
        let component = ComponentContract::new(
            ComponentName::parse("benchmark.a8_16")?,
            ContentDigest::from_bytes(&[0x20; 32])?,
            1,
            1,
            1,
        )?;
        let build_id = BuildId::parse("benchmark-build-v1")?;
        let route = RouteIdentity::from_bytes(&[0x30; 32])?;
        let slot = IslandSlot::parse("snapshot-budget")?;
        let scope = ScopeFingerprint::from_bytes(&[0x40; 32])?;
        let state = exact_state()?;
        let state_value = dehydrate(
            &state,
            schemas.state(),
            StateExposure::Instanced,
            limits.input(),
        )?;
        let memo_value = dehydrate(
            &BenchmarkMemo {},
            schemas.memo(),
            StateExposure::Instanced,
            limits.input(),
        )?;
        let fields = InstanceFieldsV1 {
            component: component.clone(),
            build_id: build_id.clone(),
            route: route.clone(),
            slot: slot.clone(),
            key_id: keys.active_key_id().clone(),
            scope: scope.clone(),
            instance_id: InstanceId::from_bytes(&[0x50; 16])?,
            revision: Revision::new(7),
            issued_at: UnixMillis::new(1_000),
            expires_at: UnixMillis::new(2_000),
            state: state_value,
            memo: memo_value,
            extensions: BTreeMap::new(),
        };
        let encoded = InstanceBodyV1::new(fields.clone(), &schemas, &limits)?.sign(
            &keys,
            UnixMillis::new(1_010),
            &limits,
        )?;
        let expected =
            ExpectedInstanceV1::new(component, build_id, route, slot, scope, schemas.clone());

        Ok(Self {
            encoded,
            fields,
            expected,
            schemas,
            limits,
            keys,
        })
    }

    fn process_once(&self) -> Result<PipelineOutput, Box<dyn Error>> {
        let verified = verify_instance(
            black_box(&self.encoded),
            &self.expected,
            &self.keys,
            UnixMillis::new(1_010),
            &self.limits,
        )?;
        let state: BenchmarkState = verified.hydrate_state(self.schemas.state())?;
        let memo: BenchmarkMemo = verified.hydrate_memo(self.schemas.memo())?;
        let dehydrated_state = dehydrate(
            black_box(&state),
            self.schemas.state(),
            StateExposure::Instanced,
            self.limits.input(),
        )?;
        let dehydrated_memo = dehydrate(
            black_box(&memo),
            self.schemas.memo(),
            StateExposure::Instanced,
            self.limits.input(),
        )?;
        let canonical_state = to_canonical_bytes(&dehydrated_state, self.limits.input())?;

        let mut fields = self.fields.clone();
        fields.revision = Revision::new(8);
        fields.issued_at = UnixMillis::new(1_011);
        fields.state = dehydrated_state;
        fields.memo = dehydrated_memo;
        let signed = InstanceBodyV1::new(fields, &self.schemas, &self.limits)?.sign(
            &self.keys,
            UnixMillis::new(1_011),
            &self.limits,
        )?;

        Ok(PipelineOutput {
            signed,
            canonical_state_bytes: canonical_state.len(),
            hydrated_state: state,
        })
    }

    fn assert_correctness_guards(&self) -> Result<(), Box<dyn Error>> {
        let expected_state = exact_state()?;
        let output = self.process_once()?;
        if output.canonical_state_bytes != STATE_BYTES || output.hydrated_state != expected_state {
            return Err(std::io::Error::other("pipeline state correctness guard failed").into());
        }
        let verified = verify_instance(
            &output.signed,
            &self.expected,
            &self.keys,
            UnixMillis::new(1_011),
            &self.limits,
        )?;
        let round_trip: BenchmarkState = verified.hydrate_state(self.schemas.state())?;
        if round_trip != expected_state {
            return Err(std::io::Error::other("signed output round trip failed").into());
        }

        let mut tampered = self.encoded.clone();
        let middle = tampered.len() / 2;
        tampered[middle] ^= 1;
        if verify_instance(
            &tampered,
            &self.expected,
            &self.keys,
            UnixMillis::new(1_010),
            &self.limits,
        )
        .is_ok()
        {
            return Err(std::io::Error::other("invalid signature guard failed").into());
        }

        let smaller_input = InputLimits::new(self.encoded.len() - 1, 12, 512, 16 * 1024)?;
        let smaller_limits = SnapshotLimits::new(smaller_input, 50, 10_000, 20_000, 8, 8)?;
        if verify_instance(
            &self.encoded,
            &self.expected,
            &self.keys,
            UnixMillis::new(1_010),
            &smaller_limits,
        )
        .is_ok()
        {
            return Err(std::io::Error::other("input-limit guard failed").into());
        }
        Ok(())
    }

    fn snapshot_overhead_bytes(&self) -> Result<usize, Box<dyn Error>> {
        let state_bytes = to_canonical_bytes(&self.fields.state, self.limits.input())?.len();
        let memo_bytes = to_canonical_bytes(&self.fields.memo, self.limits.input())?.len();
        self.encoded
            .len()
            .checked_sub(state_bytes + memo_bytes)
            .ok_or_else(|| std::io::Error::other("snapshot overhead underflow").into())
    }

    fn control_overhead_bytes(&self) -> Result<usize, Box<dyn Error>> {
        let html = "h".repeat(HTML_BYTES);
        let snapshot = std::str::from_utf8(&self.encoded)?;
        let response = format!(
            r#"{{"accepted_revision":"8","correlation_id":"EBESExQVFhcYGRobHB0eHw","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"html":"{html}","kind":"html"}},"snapshot":{snapshot},"validation":{{}}}}"#,
        );
        response
            .len()
            .checked_sub(html.len() + snapshot.len())
            .ok_or_else(|| std::io::Error::other("control overhead underflow").into())
    }
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
            database: "not_used_by_snapshot_processing",
            provider_versions: BTreeMap::from([("snapshot_key_ring", "in_process_v1")]),
            dedicated_vcpus_attested: dedicated,
            warm_filesystem_cache: true,
            loopback_providers: true,
            s1_requirements_met: requirements_met,
        }
    }
}

fn exact_state() -> Result<BenchmarkState, Box<dyn Error>> {
    let empty = BenchmarkState {
        payload: String::new(),
    };
    let framing = serde_json::to_vec(&empty)?.len();
    if framing >= STATE_BYTES {
        return Err(std::io::Error::other("state framing exceeds fixture size").into());
    }
    let state = BenchmarkState {
        payload: "x".repeat(STATE_BYTES - framing),
    };
    if serde_json::to_vec(&state)?.len() != STATE_BYTES {
        return Err(std::io::Error::other("state fixture is not exactly 8 KiB").into());
    }
    Ok(state)
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    let rank = (percentile * samples.len() as f64).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_BENCH_RESULT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("benchmarks/snapshot-budget-v1.json"))
}

fn write_result(result: &BenchmarkResult, path: PathBuf) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(&path);
    let mut bytes = serde_json::to_vec_pretty(result)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
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

fn cpu_list(encoded: &str) -> Vec<usize> {
    let mut cpus = Vec::new();
    for group in encoded.split(',') {
        if let Some((start, end)) = group.split_once('-') {
            let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
                continue;
            };
            if start <= end && end.saturating_sub(start) <= 4_096 {
                cpus.extend(start..=end);
            }
        } else if let Ok(cpu) = group.parse::<usize>() {
            cpus.push(cpu);
        }
    }
    cpus
}

fn governors(affinity: &str) -> String {
    let mut values = cpu_list(affinity)
        .into_iter()
        .filter_map(|cpu| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .ok()
            .map(|value| value.trim().to_owned())
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    match values.as_slice() {
        [] => "unavailable".to_owned(),
        [only] => only.clone(),
        _ => values.join(","),
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
