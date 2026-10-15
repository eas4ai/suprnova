//! Judged retrieval ties engine quality and warm latency to committed rival measurements.
mod calibration;
pub use calibration::Calibration;

use super::{
    AttributeDecl, AttributeType, AttributeValue, EmbedInput, Embedder, EngineQuery, GenerationId,
    IndexDescriptor, IndexSettings, IndexedDocument, ModelProfile, ModelProvenance, QueryText,
    SearchConfig, SearchDocument, SearchEngine, SearchableDecl, expand, validate_embedding,
    weighted_rrf,
};
use crate::FrameworkError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
};

const INDEX: &str = "search_baseline";
const HYBRID_RATIO: f32 = 0.5;
const LATENCY_SCOPE: &str = "warm engine query: parsing, expansion, retrieval and fusion; query inference timed separately; fixed vocabulary; no hydration";
const MAX_FIXTURE_BYTES: usize = 10_000_000;
static DATABASE_VECTORS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Rival release binaries require comparable optimized engine measurements.
/// The gate runs the baseline tests under `--cargo-profile release` for the budget.
pub fn optimized_build() -> bool {
    !cfg!(debug_assertions)
}

/// Relevance floors apply in every build; latency budgets require comparable builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetCheck {
    /// Fail when the engine's p95 exceeds the committed budget.
    Enforce,
    /// Validate and report timing without enforcing the budget in an unoptimized build.
    Report,
}
impl BudgetCheck {
    /// The gate uses `--cargo-profile release` to compare with rival release binaries.
    /// Development builds report timing while still checking relevance floors.
    pub fn for_build() -> Self {
        if optimized_build() {
            Self::Enforce
        } else {
            Self::Report
        }
    }
}

/// Corpus projections are identical for framework and rival indexes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureDocument {
    /// Source IDs connect ranked hits to judgments without renumbering.
    pub id: String,
    /// Titles are a separate searchable field on every engine.
    pub title: String,
    /// Bodies retain the upstream abstract text.
    pub text: String,
}
/// Only judged queries enter the macro average.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureQuery {
    /// Stable IDs join queries to the test qrels.
    pub id: String,
    /// Original text reaches every engine unchanged.
    pub text: String,
}
/// Validated references prevent a missing judged document from silently lowering scores.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// The complete committed corpus is indexed before any query runs.
    pub corpus: Vec<FixtureDocument>,
    /// Query order is stable across warm-up and measured passes.
    pub queries: Vec<FixtureQuery>,
    /// Nonnegative linear gains follow trec_eval's ndcg_cut convention.
    pub qrels: BTreeMap<String, BTreeMap<String, i32>>,
    /// Attribution travels with the data rather than relying on an external URL.
    pub license: String,
    /// A digest binds the exact corpus, query, judgment and license bytes.
    pub sha256: String,
}

fn file_error(path: &Path, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::param(format!("search baseline {}: {error}", path.display()))
}
fn read(path: &Path) -> Result<Vec<u8>, FrameworkError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| file_error(path, e))?;
    let mut bytes = Vec::new();
    file.take(MAX_FIXTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| file_error(path, e))?;
    if bytes.len() > MAX_FIXTURE_BYTES {
        return Err(file_error(path, "exceeds 10 MB"));
    }
    Ok(bytes)
}
fn json_lines<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<Vec<T>, FrameworkError> {
    let text = std::str::from_utf8(bytes).map_err(|e| file_error(path, e))?;
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str(line).map_err(|e| file_error(path, format!("line {}: {e}", i + 1)))
        })
        .collect()
}
impl Fixture {
    /// Refuse ambiguous IDs, malformed rows and judgments outside the committed collection.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, FrameworkError> {
        let directory = directory.as_ref();
        let mut files = Vec::new();
        let mut hash = Sha256::new();
        let mut total = 0;
        for name in ["corpus.jsonl", "queries.jsonl", "qrels.tsv", "LICENSE"] {
            let bytes = read(&directory.join(name))?;
            total += bytes.len();
            if total > MAX_FIXTURE_BYTES {
                return Err(file_error(directory, "fixture exceeds 10 MB"));
            }
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
            files.push(bytes);
        }
        let corpus: Vec<FixtureDocument> = json_lines(&directory.join("corpus.jsonl"), &files[0])?;
        let queries: Vec<FixtureQuery> = json_lines(&directory.join("queries.jsonl"), &files[1])?;
        let documents = corpus
            .iter()
            .map(|d| d.id.as_str())
            .collect::<BTreeSet<_>>();
        let query_ids = queries
            .iter()
            .map(|q| q.id.as_str())
            .collect::<BTreeSet<_>>();
        if corpus.is_empty()
            || documents.len() != corpus.len()
            || corpus
                .iter()
                .any(|d| d.id.trim().is_empty() || d.text.trim().is_empty())
        {
            return Err(file_error(
                &directory.join("corpus.jsonl"),
                "requires unique nonempty IDs and document text",
            ));
        }
        if queries.is_empty()
            || query_ids.len() != queries.len()
            || queries
                .iter()
                .any(|q| q.id.trim().is_empty() || q.text.trim().is_empty())
        {
            return Err(file_error(
                &directory.join("queries.jsonl"),
                "requires unique nonempty IDs and query text",
            ));
        }
        let path = directory.join("qrels.tsv");
        let mut rows = std::str::from_utf8(&files[2])
            .map_err(|e| file_error(&path, e))?
            .lines();
        if rows.next() != Some("query_id\tdocument_id\trelevance") {
            return Err(file_error(
                &path,
                "expected query_id, document_id, relevance TSV header",
            ));
        }
        let mut qrels = BTreeMap::<String, BTreeMap<String, i32>>::new();
        for (i, row) in rows.enumerate() {
            let columns = row.split('\t').collect::<Vec<_>>();
            if columns.len() != 3 {
                return Err(file_error(
                    &path,
                    format!("line {} requires three columns", i + 2),
                ));
            }
            let relevance = columns[2]
                .parse::<i32>()
                .ok()
                .filter(|r| *r >= 0)
                .ok_or_else(|| {
                    file_error(
                        &path,
                        format!("line {} requires nonnegative integer relevance", i + 2),
                    )
                })?;
            if !query_ids.contains(columns[0]) || !documents.contains(columns[1]) {
                return Err(file_error(
                    &path,
                    format!("line {} references an absent query or document", i + 2),
                ));
            }
            if qrels
                .entry(columns[0].into())
                .or_default()
                .insert(columns[1].into(), relevance)
                .is_some()
            {
                return Err(file_error(
                    &path,
                    format!("line {} repeats a judgment", i + 2),
                ));
            }
        }
        if queries.iter().any(|q| {
            !qrels
                .get(&q.id)
                .is_some_and(|rels| rels.values().any(|r| *r > 0))
        }) {
            return Err(file_error(
                &path,
                "every query requires a positive judgment",
            ));
        }
        let license = String::from_utf8(files.remove(3))
            .map_err(|e| file_error(&directory.join("LICENSE"), e))?;
        if license.trim().is_empty() {
            return Err(file_error(
                &directory.join("LICENSE"),
                "license must not be empty",
            ));
        }
        Ok(Self {
            corpus,
            queries,
            qrels,
            license,
            sha256: hex::encode(hash.finalize()),
        })
    }
}

/// Linear gain matches BEIR's trec_eval nDCG; repeated hits cannot inflate the score.
pub fn ndcg_at_10(
    ranked: &[String],
    judgments: &BTreeMap<String, i32>,
) -> Result<f64, FrameworkError> {
    if judgments.values().any(|r| *r < 0) {
        return Err(FrameworkError::param(
            "search baseline qrels.tsv: negative relevance",
        ));
    }
    let mut ideal = judgments.values().copied().collect::<Vec<_>>();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let ideal = ideal
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, r)| f64::from(*r) / ((i + 2) as f64).log2())
        .sum::<f64>();
    if ideal == 0.0 {
        return Ok(0.0);
    }
    let mut seen = BTreeSet::new();
    let dcg = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, key)| seen.insert(key.as_str()))
        .map(|(i, key)| f64::from(*judgments.get(key).unwrap_or(&0)) / ((i + 2) as f64).log2())
        .sum::<f64>();
    Ok(dcg / ideal)
}

/// Nearest-rank percentiles describe a complete warm query pass in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latency {
    /// Median latency distinguishes normal work from the tail.
    pub p50_ms: f64,
    /// The gate compares this tail against its committed engine budget.
    pub p95_ms: f64,
    /// Extreme tail remains visible even though the gate budgets p95.
    pub p99_ms: f64,
}
impl Latency {
    /// Reject empty, negative or nonfinite samples before publishing numbers.
    pub fn from_samples(samples: &[f64]) -> Result<Self, FrameworkError> {
        if samples.is_empty() || samples.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(FrameworkError::param(
                "search baseline latency requires finite nonnegative samples",
            ));
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let percentile = |p: usize| sorted[(p * sorted.len()).div_ceil(100) - 1];
        Ok(Self {
            p50_ms: percentile(50),
            p95_ms: percentile(95),
            p99_ms: percentile(99),
        })
    }
    fn validate(&self) -> Result<(), FrameworkError> {
        if !self.p50_ms.is_finite()
            || !self.p95_ms.is_finite()
            || !self.p99_ms.is_finite()
            || self.p50_ms < 0.0
            || self.p50_ms > self.p95_ms
            || self.p95_ms > self.p99_ms
        {
            return Err(FrameworkError::param("search baseline: invalid latency"));
        }
        Ok(())
    }
}
/// Quality and timing share the same measured pass after a full warm-up pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    /// Macro average over every judged fixture query.
    pub ndcg_at_10: f64,
    /// One engine sample per query, including expansion but excluding query inference.
    pub latency: Latency,
    /// Sample count makes accidental query omissions visible.
    pub queries: usize,
}
impl Measurement {
    fn validate(&self) -> Result<(), FrameworkError> {
        if self.queries == 0
            || !self.ndcg_at_10.is_finite()
            || !(0.0..=1.0).contains(&self.ndcg_at_10)
        {
            return Err(FrameworkError::param(
                "search baseline.json: invalid measurement",
            ));
        }
        self.latency.validate()
    }
}
/// Unavailable rival measurements are null, never guessed scores or versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rival {
    /// The version endpoint identifies the service that produced the score.
    pub version: Option<String>,
    /// Lexical measurements have no dependency on a rival embedding provider.
    pub lexical: Option<Measurement>,
    /// Search parameters of the chosen trial, excluding each fixture query's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    /// Every measured configuration makes the calibration choice reviewable.
    #[serde(default)]
    pub grid: Vec<RivalTrial>,
}
/// Search-time settings and their measured quality and warm p95 on the fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RivalTrial {
    /// Exact search parameters, excluding each fixture query's text.
    pub settings: serde_json::Value,
    /// Macro average over every judged fixture query.
    pub ndcg_at_10: f64,
    /// Warm nearest-rank p95 in milliseconds from the measured pass.
    pub p95_ms: f64,
}
impl Rival {
    fn validate_calibrated(&self) -> Result<(), FrameworkError> {
        if !self.version.as_ref().is_some_and(|v| !v.trim().is_empty()) {
            return Err(FrameworkError::param(
                "search baseline.json: calibrated rival lacks its version",
            ));
        }
        let measurement = self.lexical.as_ref().ok_or_else(|| {
            FrameworkError::param(
                "search baseline.json: calibrated rival lacks lexical measurement",
            )
        })?;
        measurement.validate()?;
        if self.settings.is_none() || self.grid.is_empty() {
            return Err(FrameworkError::param(
                "search baseline.json: calibrated rival requires chosen settings and a non-empty grid",
            ));
        }
        let mut best = 0.0_f64;
        for trial in &self.grid {
            if !trial.ndcg_at_10.is_finite()
                || !(0.0..=1.0).contains(&trial.ndcg_at_10)
                || !trial.p95_ms.is_finite()
                || trial.p95_ms < 0.0
            {
                return Err(FrameworkError::param(
                    "search baseline.json: invalid rival grid measurement",
                ));
            }
            best = best.max(trial.ndcg_at_10);
        }
        if (measurement.ndcg_at_10 - best).abs() > best * 1e-9 {
            return Err(FrameworkError::param(
                "search baseline.json: rival nDCG@10 must equal its best grid trial",
            ));
        }
        Ok(())
    }
}
/// Each rival is retained separately so a weak rival cannot lower the gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rivals {
    /// Meilisearch uses title and text in that searchable order.
    pub meilisearch: Rival,
    /// Typesense uses query_by=title,text.
    pub typesense: Rival,
}
/// Calibration derives relevance floors and engine budgets from the same rival pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineFloors {
    /// False closes the normal gate until the developer runs calibration.
    pub calibrated: bool,
    /// Exact data and license bytes that both rivals indexed.
    pub fixture_sha256: String,
    /// Both licenses are recorded alongside the scores.
    pub fixture_license: String,
    /// Absolute nDCG allowance below the stronger rival in lexical mode.
    pub tolerance: f64,
    /// Scores and actual service versions, absent before calibration.
    pub rivals: Rivals,
    /// The timing boundary prevents a changed benchmark from silently reusing old budgets.
    pub latency_scope: String,
    /// Per-engine warm p95 limits in milliseconds, committed and reviewable.
    pub p95_budgets_ms: BTreeMap<String, f64>,
    /// Evidence and headroom explain how the budgets were chosen.
    pub budget_comment: String,
    /// Calibration time is absent until real services have answered.
    pub calibrated_at: Option<String>,
    /// The checkout that performed calibration, when git is available.
    pub tree_hash: Option<String>,
}
impl BaselineFloors {
    /// Validate committed numbers before they can influence a gate.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FrameworkError> {
        let path = path.as_ref();
        let value: Self = serde_json::from_slice(&read(path)?).map_err(|e| file_error(path, e))?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), FrameworkError> {
        if !self.tolerance.is_finite()
            || !(0.0..=1.0).contains(&self.tolerance)
            || self.fixture_sha256.len() != 64
            || !self.fixture_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.fixture_license.trim().is_empty()
            || self.budget_comment.trim().is_empty()
            || self.latency_scope != LATENCY_SCOPE
            || self.p95_budgets_ms.is_empty()
            || self
                .p95_budgets_ms
                .values()
                .any(|b| !b.is_finite() || *b <= 0.0)
        {
            return Err(FrameworkError::param(
                "search baseline.json: invalid fixture digest, tolerance, license or p95 budgets",
            ));
        }
        if self.calibrated {
            for rival in [&self.rivals.meilisearch, &self.rivals.typesense] {
                rival.validate_calibrated()?;
            }
            let derived = self.rival_budget()?;
            if self
                .p95_budgets_ms
                .values()
                .any(|budget| (*budget - derived).abs() / derived > 1e-9)
            {
                return Err(FrameworkError::param(
                    "search baseline.json: p95 budgets must be two times the slower rival's p95",
                ));
            }
            if self
                .calibrated_at
                .as_ref()
                .is_none_or(|s| chrono::DateTime::parse_from_rfc3339(s).is_err())
            {
                return Err(FrameworkError::param(
                    "search baseline.json: calibrated_at must be RFC3339",
                ));
            }
        }
        Ok(())
    }
    /// An explicit development override is the only way to run a provisional gate.
    pub fn require_calibrated(&self, allow_uncalibrated: bool) -> Result<(), FrameworkError> {
        self.validate()?;
        if !self.calibrated && !allow_uncalibrated {
            return Err(FrameworkError::param(
                "search baseline.json is uncalibrated: run search:baseline calibrate; provisional measurements require SEARCH_BASELINE_ALLOW_UNCALIBRATED=1",
            ));
        }
        Ok(())
    }
    /// Persist a fully validated replacement atomically, leaving old floors on any failure.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), FrameworkError> {
        self.validate()?;
        write_json(path.as_ref(), self)
    }
    fn rival_score(&self) -> Result<f64, FrameworkError> {
        let score = |r: &Rival| {
            r.lexical.as_ref().map(|m| m.ndcg_at_10).ok_or_else(|| {
                FrameworkError::param("search baseline.json: rival measurement is absent")
            })
        };
        Ok(score(&self.rivals.meilisearch)?.max(score(&self.rivals.typesense)?))
    }
    fn rival_budget(&self) -> Result<f64, FrameworkError> {
        let p95 = |r: &Rival| {
            r.lexical.as_ref().map(|m| m.latency.p95_ms).ok_or_else(|| {
                FrameworkError::param("search baseline.json: rival measurement is absent")
            })
        };
        let budget = 2.0 * p95(&self.rivals.meilisearch)?.max(p95(&self.rivals.typesense)?);
        if !budget.is_finite() || budget <= 0.0 {
            return Err(FrameworkError::param(
                "search baseline.json: rival-derived p95 budget must be finite and positive",
            ));
        }
        Ok(budget)
    }
}

/// Every run keeps enough identity to compare results without relying on console logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineRecord {
    /// Concrete backend names distinguish SQLite, Postgres and MySQL.
    pub engine: String,
    /// None explicitly identifies lexical-only runs without an embedding provider.
    pub model_profile: Option<ModelProfile>,
    /// Exact model revision, manifest and file hashes accompany hybrid results.
    pub model_provenance: Option<ModelProvenance>,
    /// Both rival versions identify the calibration behind the floors.
    pub rivals: Rivals,
    /// Provisional results cannot be confused with calibrated passes.
    pub calibrated: bool,
    /// Git HEAD is retained when the checkout has git metadata.
    pub tree_hash: Option<String>,
    /// Fixture identity catches comparisons made across changed corpora or judgments.
    pub fixture_sha256: String,
    /// RFC3339 UTC timestamp identifies the measured run.
    pub measured_at: String,
    /// Explicit boundaries distinguish warm engine timing from hydrated builder latency.
    pub latency_scope: String,
    /// Snapshot preparation remains visible although it is outside the warm query samples.
    pub vocabulary_prepare_ms: f64,
    /// The engine's lexical ranking under the builder's default expansion.
    pub lexical: Measurement,
    /// Real-provider hybrid ranking and latency, absent for lexical-only runs.
    pub hybrid: Option<Measurement>,
    /// The model's warm query inference cost is separate from the engine's budget.
    pub query_inference: Option<Latency>,
    /// Explicit benchmark default avoids mislabeling the core lexical ratio as hybrid.
    pub hybrid_ratio: f32,
}
impl BaselineRecord {
    /// Write evidence before evaluating floors so failed runs remain inspectable.
    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), FrameworkError> {
        write_json(path.as_ref(), self)
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), FrameworkError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| file_error(parent, e))?;
    let temp = parent.join(format!(".baseline_{}.json", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| file_error(&temp, e))?;
        serde_json::to_writer_pretty(&mut file, value).map_err(|e| file_error(path, e))?;
        file.write_all(b"\n").map_err(|e| file_error(path, e))?;
        file.sync_all().map_err(|e| file_error(path, e))?;
        std::fs::rename(&temp, path).map_err(|e| file_error(path, e))
    })();
    match result {
        Ok(()) => Ok(()),
        Err(error) => match std::fs::remove_file(&temp) {
            Ok(()) => Err(error),
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => Err(error),
            Err(cleanup) => Err(FrameworkError::internal(format!(
                "{error}; search baseline temporary file cleanup: {cleanup}"
            ))),
        },
    }
}
fn tree_hash() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The runner owns the fixture and floors while engines keep their native ranking.
pub struct Baseline {
    fixture: Fixture,
    floors: BaselineFloors,
}
impl Baseline {
    /// Load fixture and floor identities together so stale calibration cannot pass.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, FrameworkError> {
        let directory = directory.as_ref();
        let fixture = Fixture::load(directory)?;
        let floors = BaselineFloors::load(directory.join("baseline.json"))?;
        if fixture.sha256 != floors.fixture_sha256 {
            return Err(file_error(
                &directory.join("baseline.json"),
                "fixture_sha256 does not match corpus, queries, qrels and LICENSE; recalibrate",
            ));
        }
        if floors.calibrated
            && [&floors.rivals.meilisearch, &floors.rivals.typesense]
                .iter()
                .any(|r| {
                    r.lexical
                        .as_ref()
                        .is_none_or(|m| m.queries != fixture.queries.len())
                })
        {
            return Err(file_error(
                &directory.join("baseline.json"),
                "rival query count does not match fixture",
            ));
        }
        Ok(Self { fixture, floors })
    }
    /// Index into an isolated inactive generation, warm every query, measure, then remove it.
    pub async fn run(
        &self,
        engine: &str,
        driver: &dyn SearchEngine,
        embedder: Option<&dyn Embedder>,
    ) -> Result<BaselineRecord, FrameworkError> {
        if !self.floors.p95_budgets_ms.contains_key(engine) {
            return Err(FrameworkError::param(format!(
                "search baseline.json: no p95 budget for engine {engine}"
            )));
        }
        // The fallback registry is process-global. Hold it through cleanup so a
        // concurrent database benchmark cannot replace another run's vectors.
        let _vectors = if driver.name() == "database"
            && embedder.is_some()
            && crate::config::env_optional::<String>("SEARCH_VECTOR_STORE").is_none()
        {
            Some(DATABASE_VECTORS.lock().await)
        } else {
            None
        };
        let index = descriptor();
        // Native SQL identifiers start with a letter; the full table name stays below 57 bytes.
        let generation = GenerationId::new(format!("b{}", uuid::Uuid::new_v4().simple()))?;
        let mut settings = index.index_settings();
        if let Some(provider) = embedder {
            provider.profile().validate()?;
            if provider.profile().id.starts_with("fake") {
                return Err(FrameworkError::param(
                    "search baseline hybrid requires a real SEARCH_EMBEDDER",
                ));
            }
            settings.index_time.dimensions = Some(provider.profile().dimensions);
            settings.index_time.model = Some(provider.profile().id.clone());
        }
        driver
            .create_generation(&index, &settings, &generation)
            .await?;
        let result = self
            .measure_engine(engine, driver, embedder, &index, &settings, &generation)
            .await;
        let cleanup = driver.drop_generation(&index, &generation).await;
        match (result, cleanup) {
            (Ok(record), Ok(())) => Ok(record),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(FrameworkError::internal(format!(
                "{error}; baseline generation cleanup: {cleanup}"
            ))),
        }
    }
    async fn measure_engine(
        &self,
        engine: &str,
        driver: &dyn SearchEngine,
        embedder: Option<&dyn Embedder>,
        index: &IndexDescriptor,
        settings: &IndexSettings,
        generation: &GenerationId,
    ) -> Result<BaselineRecord, FrameworkError> {
        if driver.name() == "database"
            && embedder.is_some()
            && crate::config::env_optional::<String>("SEARCH_VECTOR_STORE").is_none()
        {
            crate::vector::Vector::register(
                INDEX,
                std::sync::Arc::new(crate::vector::memory::MemoryVectorDriver::new()),
            );
        }
        let config = SearchConfig::current()?;
        for chunk in self.fixture.corpus.chunks(config.chunk) {
            let inputs = chunk
                .iter()
                .map(|d| EmbedInput {
                    text: d.text.clone(),
                    title: Some(d.title.clone()),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let vectors = match embedder {
                Some(provider) => provider.embed_documents(&inputs).await?,
                None => vec![],
            };
            if embedder.is_some() && vectors.len() != chunk.len() {
                return Err(FrameworkError::param(
                    "SEARCH_EMBEDDER: baseline document embedding count differs from corpus chunk",
                ));
            }
            let mut documents = Vec::with_capacity(chunk.len());
            for (i, source) in chunk.iter().enumerate() {
                let document = SearchDocument {
                    index: INDEX.into(),
                    key: source.id.clone(),
                    source_version: 1,
                    attributes: BTreeMap::from([
                        ("title".into(), AttributeValue::Text(source.title.clone())),
                        ("text".into(), AttributeValue::Text(source.text.clone())),
                    ]),
                    embed_text: Some(source.text.clone()),
                    title: Some(source.title.clone()),
                };
                let dense = match embedder {
                    Some(provider) => Some(
                        validate_embedding(
                            vectors[i].dense.clone(),
                            provider.profile(),
                            settings.index_time.dimensions,
                        )?
                        .dense,
                    ),
                    None => None,
                };
                documents.push(IndexedDocument {
                    content_hash: document.content_hash(&settings.index_time.digest()),
                    document,
                    dense,
                    sparse: None,
                });
            }
            driver.index(index, generation, &documents).await?;
        }
        // The generation is inactive and immutable for both measured modes.
        let prepare_started = Instant::now();
        let vocabulary = driver.vocabulary(index, generation).await?;
        let vocabulary_prepare_ms = prepare_started.elapsed().as_secs_f64() * 1000.0;
        let vocabulary = vocabulary.as_ref();
        let run = |query: FixtureQuery, hybrid: bool| async move {
            let (vector, query_inference_ms) = if let Some(provider) = embedder.filter(|_| hybrid) {
                let start = Instant::now();
                let vector = provider.embed_query(&query.text).await?;
                let inference_ms = start.elapsed().as_secs_f64() * 1000.0;
                let vector = validate_embedding(
                    vector.dense,
                    provider.profile(),
                    settings.index_time.dimensions,
                )?;
                (Some(vector), Some(inference_ms))
            } else {
                (None, None)
            };
            let start = Instant::now();
            let expanded = expand(
                &QueryText::parse(&query.text),
                &settings.query,
                vocabulary,
                config.expansion_terms,
            )?;
            let plan = EngineQuery {
                expanded,
                filter: None,
                sort: vec![],
                depth: if hybrid {
                    settings.query.fusion_depth
                } else {
                    settings.query.hits_cap
                },
                highlights: false,
                crop: BTreeMap::new(),
            };
            let lexical = driver.query(index, generation, &plan).await?;
            let keys = |hits: super::EngineResult| {
                hits.candidates
                    .into_iter()
                    .map(|c| c.document.key)
                    .collect::<Vec<_>>()
            };
            let finish = |keys| {
                Ok(QuerySample {
                    keys,
                    engine_ms: start.elapsed().as_secs_f64() * 1000.0,
                    query_inference_ms,
                })
            };
            let Some(vector) = vector else {
                return finish(keys(lexical));
            };
            if let Some(mut native) = driver
                .hybrid(index, generation, &plan, &vector.dense, HYBRID_RATIO)
                .await?
            {
                native.candidates.retain(|c| {
                    !super::builder::matches_negative(
                        &c.document,
                        settings,
                        &plan.expanded.negatives,
                    )
                });
                return finish(keys(native));
            }
            let mut semantic = driver
                .semantic(
                    index,
                    generation,
                    &vector.dense,
                    None,
                    settings.query.fusion_depth,
                )
                .await?;
            semantic.candidates.retain(|c| {
                !super::builder::matches_negative(&c.document, settings, &plan.expanded.negatives)
            });
            finish(
                weighted_rrf(&[
                    (1.0 - f64::from(HYBRID_RATIO), keys(lexical)),
                    (f64::from(HYBRID_RATIO), keys(semantic)),
                ])
                .into_iter()
                .map(|(key, _)| key)
                .collect(),
            )
        };
        let (lexical, _) = measure_queries(&self.fixture, |q| run(q, false)).await?;
        let (hybrid, query_inference) = if embedder.is_some() {
            let (measurement, inference) = measure_queries(&self.fixture, |q| run(q, true)).await?;
            (Some(measurement), inference)
        } else {
            (None, None)
        };
        Ok(BaselineRecord {
            engine: engine.into(),
            model_profile: embedder.map(|e| e.profile().clone()),
            model_provenance: embedder.map(|e| e.profile().provenance()).transpose()?,
            rivals: self.floors.rivals.clone(),
            calibrated: self.floors.calibrated,
            tree_hash: tree_hash(),
            fixture_sha256: self.fixture.sha256.clone(),
            measured_at: crate::clock::now().to_rfc3339(),
            latency_scope: LATENCY_SCOPE.into(),
            vocabulary_prepare_ms,
            lexical,
            hybrid,
            query_inference,
            hybrid_ratio: HYBRID_RATIO,
        })
    }
    /// Apply relevance floors in every build and the engine budget when requested.
    /// The gate uses `--cargo-profile release` for latency comparisons with rival binaries.
    pub fn assert_record(
        &self,
        record: &BaselineRecord,
        allow_uncalibrated: bool,
        budget_check: BudgetCheck,
    ) -> Result<(), FrameworkError> {
        self.floors.require_calibrated(allow_uncalibrated)?;
        if record.latency_scope != self.floors.latency_scope
            || !record.vocabulary_prepare_ms.is_finite()
            || record.vocabulary_prepare_ms < 0.0
        {
            return Err(FrameworkError::param(
                "search baseline record has an incompatible latency scope or vocabulary preparation time",
            ));
        }
        if record.fixture_sha256 != self.fixture.sha256 {
            return Err(FrameworkError::param(
                "search baseline record fixture identity differs",
            ));
        }
        if record.calibrated != self.floors.calibrated
            || record.rivals.meilisearch.version != self.floors.rivals.meilisearch.version
            || record.rivals.typesense.version != self.floors.rivals.typesense.version
        {
            return Err(FrameworkError::param(
                "search baseline record uses a different calibration",
            ));
        }
        if record.hybrid.is_some()
            && (record.model_profile.is_none()
                || record.model_provenance.is_none()
                || record.hybrid_ratio != HYBRID_RATIO)
        {
            return Err(FrameworkError::param(
                "search baseline hybrid record requires its model profile, provenance and benchmark ratio",
            ));
        }
        if record.hybrid.is_some() != record.query_inference.is_some() {
            return Err(FrameworkError::param(
                "search baseline query_inference is required only for hybrid records",
            ));
        }
        if let Some(inference) = &record.query_inference {
            inference.validate()?;
        }
        let budget = self
            .floors
            .p95_budgets_ms
            .get(&record.engine)
            .ok_or_else(|| {
                FrameworkError::param(format!(
                    "search baseline.json: missing {} p95 budget",
                    record.engine
                ))
            })?;
        for (mode, measurement) in [
            ("lexical", Some(&record.lexical)),
            ("hybrid", record.hybrid.as_ref()),
        ] {
            let Some(measurement) = measurement else {
                continue;
            };
            measurement.validate()?;
            if measurement.queries != self.fixture.queries.len() {
                return Err(FrameworkError::param(
                    "search baseline record query count differs",
                ));
            }
            if budget_check == BudgetCheck::Report {
                crate::console::line(format!(
                    "search baseline {} {mode}: Report p95 {:.3} ms, budget {budget:.3} ms; budget not enforced: BudgetCheck::Report selected; comparable rival latency requires an optimized build",
                    record.engine, measurement.latency.p95_ms
                ));
            } else if measurement.latency.p95_ms > *budget {
                return Err(FrameworkError::param(format!(
                    "search baseline {} {mode} p95 {:.3} ms exceeds {budget:.3} ms",
                    record.engine, measurement.latency.p95_ms
                )));
            }
            if self.floors.calibrated {
                let rival = self.floors.rival_score()?;
                if (mode == "lexical" && measurement.ndcg_at_10 < rival - self.floors.tolerance)
                    || (mode == "hybrid" && measurement.ndcg_at_10 <= rival)
                {
                    return Err(FrameworkError::param(format!(
                        "search baseline {} {mode} nDCG@10 {:.6} misses rival {rival:.6} (lexical tolerance {})",
                        record.engine, measurement.ndcg_at_10, self.floors.tolerance
                    )));
                }
            }
        }
        Ok(())
    }
    /// Print the committed floors alongside records without exposing credentials or URLs.
    pub fn print(&self, record: &BaselineRecord) -> Result<(), FrameworkError> {
        crate::console::line(
            serde_json::to_string_pretty(
                &serde_json::json!({"record": record, "floors": self.floors}),
            )
            .map_err(|e| FrameworkError::internal(format!("search baseline record: {e}")))?,
        );
        Ok(())
    }
}

struct QuerySample {
    keys: Vec<String>,
    engine_ms: f64,
    query_inference_ms: Option<f64>,
}

async fn measure<F, Fut>(fixture: &Fixture, mut query: F) -> Result<Measurement, FrameworkError>
where
    F: FnMut(FixtureQuery) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, FrameworkError>>,
{
    let (measurement, _) = measure_queries(fixture, |q| {
        let start = Instant::now();
        let result = query(q);
        async move {
            let keys = result.await?;
            Ok(QuerySample {
                keys,
                engine_ms: start.elapsed().as_secs_f64() * 1000.0,
                query_inference_ms: None,
            })
        }
    })
    .await?;
    Ok(measurement)
}

async fn measure_queries<F, Fut>(
    fixture: &Fixture,
    mut query: F,
) -> Result<(Measurement, Option<Latency>), FrameworkError>
where
    F: FnMut(FixtureQuery) -> Fut,
    Fut: std::future::Future<Output = Result<QuerySample, FrameworkError>>,
{
    for q in &fixture.queries {
        query(q.clone()).await?;
    }
    let mut samples = Vec::with_capacity(fixture.queries.len());
    let mut inference = Vec::with_capacity(fixture.queries.len());
    let mut score = 0.0;
    for q in &fixture.queries {
        let sample = query(q.clone()).await?;
        samples.push(sample.engine_ms);
        if let Some(ms) = sample.query_inference_ms {
            inference.push(ms);
        }
        let judgments = fixture
            .qrels
            .get(&q.id)
            .ok_or_else(|| FrameworkError::param("search baseline qrels.tsv: missing query"))?;
        score += ndcg_at_10(&sample.keys, judgments)?;
    }
    let query_inference = if inference.is_empty() {
        None
    } else if inference.len() == fixture.queries.len() {
        Some(Latency::from_samples(&inference)?)
    } else {
        return Err(FrameworkError::param(
            "search baseline: incomplete query inference samples",
        ));
    };
    Ok((
        Measurement {
            ndcg_at_10: score / fixture.queries.len() as f64,
            latency: Latency::from_samples(&samples)?,
            queries: fixture.queries.len(),
        },
        query_inference,
    ))
}

pub(super) fn descriptor() -> IndexDescriptor {
    IndexDescriptor {
        name: INDEX,
        model_table: INDEX,
        key_column: "id",
        attributes: ["title", "text"]
            .iter()
            .enumerate()
            .map(|(i, name)| AttributeDecl {
                name: (*name).into(),
                ty: AttributeType::Text,
                searchable: Some(SearchableDecl {
                    weight: 1.0,
                    order: i,
                }),
                filterable: false,
                sortable: false,
                facet: false,
            })
            .collect(),
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
    }
}

/// The console and tests use the same source fixture directory to prevent data drift.
pub(super) fn fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/search/fixtures")
}
