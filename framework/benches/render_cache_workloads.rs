//! The four framework RenderCache workloads, measured on demand: the
//! middleware hot hit, the batched generation reread, an invalidation
//! storm, and two nodes over one backend. Never a gate step. See
//! `crates/suprnova-live/scripts/run-render-cache-budget.sh`, which runs
//! this beside the engine budget bench and then the checked-result
//! contract.
//!
//! Each workload reports numbers *and* asserts the correctness condition
//! those numbers are only meaningful beside, so a run that got fast by
//! getting wrong fails instead of reporting. The four conditions are: a
//! lease-mode hot hit issues no statement; a coherence reread is one
//! batched statement, never a generation read plus an epoch read; a write
//! storm rebuilds each key once per burst and leaves every key serving the
//! generation the storm ended on; and sixty-four concurrent cold requests
//! across two nodes publish exactly once.
//!
//! # What the timings include, and what they do not
//!
//! `c64_middleware` reports two latencies for the same requests.
//! `p50_microseconds` / `p95_microseconds` are the **server side**: the
//! whole of `handle_request`, from the moment hyper hands the parsed
//! request to the router to the moment the response value exists, recorded
//! inside the test host's own service through
//! `render_cache_middleware_support::recording::dispatch_get_timed`. They
//! exclude the connection, the response write, and the client's read.
//! `round_trip_p50_microseconds` / `round_trip_p95_microseconds` are the
//! **whole loopback request** measured around that same call: a fresh TCP
//! connection, an HTTP/1.1 exchange, and the body collected back. The
//! `transport` field names which transport that second pair paid for. The
//! engine's own `render_cache_budget` bench is where a hot hit is measured
//! with neither a socket nor a router in the way.
//!
//! `invalidation_storm` reports `quiescent_hit_p95_microseconds`, not a
//! hit latency during the writes, and the name says so. Every render on the
//! storm route calls `Model::find`, which observes the row it hydrated and
//! the `posts` table's unkeyed-write identity, never the table itself
//! (`framework/src/eloquent/model.rs`), so a row-level write invalidates
//! only the keys that read that row. `point_read_invalidation_ratio`
//! measures exactly that fraction over all sixty-four keys, and
//! `every_write_invalidates_every_key` keeps measuring the older question
//! rather than assuming either answer. The bursts below still write every
//! one of the twelve rows, so every key does rebuild once per burst, and
//! the hit latency reported is the one that follows a rebuild.
//!
//! `multi_node` reports `fan_in_p95_microseconds` over hand-driven
//! coordinator calls - admission, and for the one leader the publication
//! that follows it - not over served requests. No router, no middleware and
//! no socket are involved; it is the coordination cost sixty-four
//! concurrent cold requests for one key would pay, and nothing else.
//! `takeover_p95_milliseconds` is likewise the coordinator and store work a
//! takeover costs, with store time moved rather than waited out.
//!
//! # Environment variables it honours
//!
//! - `SUPRNOVA_LIVE_WORKLOADS_RESULT` redirects the result file; without
//!   it a release run writes the checked-in
//!   `crates/suprnova-live/benchmarks/render-cache-workloads-v1.json`.
//! - `PG_TEST_URL`, when set, runs the reread and multi-node workloads
//!   again against that PostgreSQL server and records a second entry under
//!   `runs`. The server must be disposable: the run drops and recreates
//!   every table it uses.
//! - `REDIS_TEST_URL`, when set, runs the multi-node workload again over
//!   the Redis lease store and Redis render store and records a third entry
//!   under `runs` with `accelerator: "redis"`. Every key it writes is
//!   under a fresh namespace of its own and is removed afterwards.
//! - `SUPRNOVA_LIVE_REQUIRE_S1=1` refuses to measure at all unless the
//!   environment is `validated_s1`, naming the conditions it refused on.
//!   Without it an unqualified machine measures and the result says
//!   `local_exploratory`, which is what a workstation run is.

// `pub`, as `framework/tests/render_cache/main.rs` declares its own test
// modules: these support modules re-export names this bench has no use for,
// and a private module would report each of them as an unused import here
// while the test binary next door uses every one.
#[path = "../tests/support/render_cache_feature_evaluator_support.rs"]
pub mod render_cache_feature_evaluator_support;
#[path = "../tests/support/render_cache_middleware_support/mod.rs"]
pub mod render_cache_middleware_support;
#[path = "../tests/support/render_cache_tiers_support/mod.rs"]
pub mod render_cache_tiers_support;

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::Serialize;
use suprnova::render_cache::providers::{
    RedisLeaseStore, RedisProviderConfig, RedisRenderStore, SqlLeaseStore, SqlRenderStore,
};
use suprnova::render_cache::{DependencyIdentity, RenderCache};
use suprnova::{DB, FrameworkError, Model, attrs};
use suprnova_live::render_cache::generation::GenerationLedger;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};
use suprnova_live::render_cache::{FencedLeaseCoordinator, LeaseStore};
use suprnova_live_test_support::bench_environment::{self, EnvironmentEvidence, percentile};

use render_cache_middleware_support::recording::{ServerTimingLog, dispatch_get_timed};
use render_cache_middleware_support::{
    C64_BODY_BYTES, C64_DEPENDENCY_ROWS, C64_ROUTE, Harness, Post, STORM_ROUTE,
    boot_with_render_cache, boot_with_render_cache_on_live_server_for_test, counting_route,
    dispatch_get, ledger, statements, storm_body_row,
};

// -------------------------------------------------------------------------
// Shape constants
// -------------------------------------------------------------------------

/// Requests every latency workload runs before it measures one.
const WARMUP: usize = 200;
/// Measured requests per latency workload; one request per sample, so
/// `samples` in the result describes the measurement completely.
const SAMPLES: usize = 200;

/// The one path the `C64` route is measured at.
const C64_PATH: &str = "/c64/1";
/// What `transport` records for the round-trip pair.
const C64_TRANSPORT: &str = "loopback_http1";

/// Dependency identities the reread carries, and the number of `posts`
/// rows the storm writes to.
const IDENTITIES: usize = 12;
/// The ceiling a reread is reported against, in milliseconds.
const REREAD_CAP_MILLISECONDS: f64 = 3.0;

/// Cached keys the storm holds.
const STORM_KEYS: usize = 64;
/// Writes the storm commits.
const STORM_WRITES: usize = 1_000;
/// How many bursts those writes are committed in.
const STORM_BURSTS: usize = 20;
/// Sweeps of every key after each burst: one that must rebuild every key,
/// one that must hit every key.
const SWEEPS_PER_BURST: usize = 2;

/// Nodes the multi-node workload runs.
const NODES: usize = 2;
/// Cold requests the multi-node workload fans in, split across the nodes.
const CONCURRENT_REQUESTS: usize = 64;
/// Takeover rounds measured, each over its own key.
const TAKEOVER_ROUNDS: usize = 40;
/// The lease lifetime the fan-in nodes hold, in milliseconds.
const FAN_IN_LEASE_MS: u64 = 30_000;
/// The lease lifetime a takeover round elapses past, in milliseconds.
const TAKEOVER_LEASE_MS: u64 = 1_000;
/// How far each takeover round moves store time past the round before it.
/// Longer than [`TAKEOVER_LEASE_MS`], so one round's move elapses the lease
/// that round took out and no round can reach a lease of an earlier one.
const TAKEOVER_OFFSET_STEP_MS: u64 = 2_000;
/// Waiters one node admits per key while its leader holds it. Above the
/// per-node request count, so every concurrent request either leads,
/// bypasses, or waits - never bypasses for want of a waiter slot.
const MAX_WAITERS: usize = 128;
/// The byte bound the multi-node render stores are built with.
const MAX_STORE_BYTES: u64 = 1024 * 1024;
/// How long a multi-node publication is retained, in milliseconds.
const PUBLICATION_RETENTION_MS: u64 = 60_000;
/// The instant a multi-node publication is stamped with.
const PUBLISHED_AT_MS: u64 = 1_000;
/// The authority epoch every multi-node admission names.
const NODE_EPOCH: u64 = 1;

/// Reduced counts for a debug build, which runs every correctness
/// assertion and skips timing (as `snapshot_budget` does) and writes no
/// result. Full counts in a debug build would take minutes to prove
/// nothing a release run does not prove.
const fn scaled(full: usize, timed: bool) -> usize {
    if timed { full } else { 2 }
}

// -------------------------------------------------------------------------
// Result record
// -------------------------------------------------------------------------

/// One run of the workloads across every database and accelerator measured.
#[derive(Serialize)]
struct WorkloadsResult {
    schema_version: u8,
    profile: &'static str,
    measured_at_unix_ms: u128,
    environment: EnvironmentEvidence,
    runs: Vec<DatabaseRun>,
}

/// The workloads one `(database, accelerator)` pair answered.
///
/// Every workload is optional and absent rather than zero when that pair
/// did not run it: `c64_middleware` and `invalidation_storm` need the
/// middleware harness, which owns its own SQLite file, and the Redis run
/// exercises no database at all.
#[derive(Serialize)]
struct DatabaseRun {
    database: &'static str,
    accelerator: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    c64_middleware: Option<C64Middleware>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_reread: Option<GenerationReread>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invalidation_storm: Option<InvalidationStorm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    multi_node: Option<MultiNode>,
}

/// A 64 KiB hot hit served through the whole middleware, timed twice: once
/// where the work happens and once around the loopback request that carried
/// it. See the module doc for exactly what each pair includes.
#[derive(Serialize)]
struct C64Middleware {
    body_bytes: usize,
    dependencies: usize,
    transport: &'static str,
    warmup: usize,
    samples: usize,
    p50_microseconds: f64,
    p95_microseconds: f64,
    round_trip_p50_microseconds: f64,
    round_trip_p95_microseconds: f64,
    statements_per_hit: u64,
}

/// The batched coherence reread over [`IDENTITIES`] dependency digests.
#[derive(Serialize)]
struct GenerationReread {
    keys: usize,
    warmup: usize,
    samples: usize,
    p50_milliseconds: f64,
    p95_milliseconds: f64,
    cap_milliseconds: f64,
    statements_per_reread: u64,
}

/// A write storm against every cached key, and what it cost in rebuilds.
///
/// Every constant that determines the ratios is recorded beside them, so a
/// reader can derive `rebuilds_per_write`'s bounds from the file without
/// reading this source.
#[derive(Serialize)]
struct InvalidationStorm {
    keys: usize,
    identities: usize,
    writes: usize,
    bursts: usize,
    writes_per_burst: usize,
    sweeps_per_burst: usize,
    every_write_invalidates_every_key: bool,
    point_read_invalidation_ratio: f64,
    hits: u64,
    rebuilds: u64,
    rebuilds_per_write: f64,
    statements_per_hit: u64,
    quiescent_hit_p95_microseconds: f64,
    final_bodies_coherent: bool,
}

/// Two nodes over one backend: the fan-in of cold requests, and what a
/// fenced leader's takeover costs.
#[derive(Serialize)]
struct MultiNode {
    nodes: usize,
    concurrent_requests: usize,
    publications: usize,
    duplicate_renders: usize,
    fan_in_p95_microseconds: f64,
    takeover_p95_milliseconds: f64,
}

// -------------------------------------------------------------------------
// Shared helpers
// -------------------------------------------------------------------------

/// The two nearest-rank percentiles over one workload's samples.
struct Timing {
    p50: f64,
    p95: f64,
}

impl Timing {
    /// Sorts the samples and takes the two percentiles.
    ///
    /// # Panics
    ///
    /// Panics when `samples` is empty; every caller measures at least one
    /// sample before asking for a percentile of them.
    fn from_samples(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        Self {
            p50: percentile(&samples, 0.50),
            p95: percentile(&samples, 0.95),
        }
    }

    /// Percentiles when there is something to take them of, and the zero
    /// record otherwise - which is what a debug profile reports, since it
    /// runs every workload and times none of them.
    fn of(samples: Vec<f64>, timed: bool) -> Self {
        if timed {
            Self::from_samples(samples)
        } else {
            Self { p50: 0.0, p95: 0.0 }
        }
    }
}

/// Microseconds since `started`.
fn elapsed_microseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000_000.0
}

/// Milliseconds since `started`.
fn elapsed_milliseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

/// Runs `once` `samples` times and returns one sample per run, scaled by
/// `scale`.
///
/// The runs happen whether or not the caller is timing: a debug profile has
/// to keep every correctness assertion inside `once` firing, and only the
/// numbers are skipped. `Timing::of` is what then discards them.
async fn measure<F, Fut>(
    samples: usize,
    scale: fn(Instant) -> f64,
    mut once: F,
) -> Result<Vec<f64>, Box<dyn Error>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
{
    let mut collected = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        once().await?;
        collected.push(scale(started));
    }
    Ok(collected)
}

/// Fails with a message naming the fact, what it was, and what it had to
/// be. Never carries a body, a key, a header value, or any SQL.
fn expect<T>(observed: T, required: T, fact: &str) -> Result<(), Box<dyn Error>>
where
    T: PartialEq + std::fmt::Debug,
{
    if observed == required {
        return Ok(());
    }
    Err(io::Error::other(format!("{fact} is {observed:?}, not {required:?}")).into())
}

/// Fails when `condition` does not hold, naming the contract.
fn require(condition: bool, fact: &str) -> Result<(), Box<dyn Error>> {
    if condition {
        return Ok(());
    }
    Err(io::Error::other(fact.to_owned()).into())
}

/// Refuses to measure when the runner demanded a qualified S1 environment
/// and this machine cannot prove one, naming the conditions it refused on.
/// The benchmark never attests S1 on its own behalf.
fn assert_required_environment(environment: &EnvironmentEvidence) -> Result<(), Box<dyn Error>> {
    if std::env::var("SUPRNOVA_LIVE_REQUIRE_S1").as_deref() != Ok("1")
        || environment.s1_requirements_met
    {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "SUPRNOVA_LIVE_REQUIRE_S1 is 1 and this run cannot prove the S1 environment: {}",
        environment.unmet_s1_requirements().join("; ")
    ))
    .into())
}

/// Where the result is written: `SUPRNOVA_LIVE_WORKLOADS_RESULT` when set,
/// otherwise the checked-in file beside the Live crate's manifest.
fn result_path() -> PathBuf {
    std::env::var_os("SUPRNOVA_LIVE_WORKLOADS_RESULT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../crates/suprnova-live/benchmarks/render-cache-workloads-v1.json")
        })
}

/// Writes through a temporary file and one rename, so a reader never sees
/// a half-written result.
fn write_result(result: &WorkloadsResult, path: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let mut bytes = serde_json::to_vec_pretty(result)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Creates `rows` `posts` rows through the ORM, returning their ids in the
/// order they were created.
///
/// # Errors
///
/// Returns whatever the ORM returns; a fresh harness database is empty, so
/// nothing here can collide.
async fn create_posts(rows: usize) -> Result<Vec<i64>, Box<dyn Error>> {
    let mut ids = Vec::with_capacity(rows);
    for index in 0..rows {
        let title = format!("row-{index}");
        ids.push(Post::create(attrs! { title: title }).await?.id);
    }
    Ok(ids)
}

/// The `views` a row holds, read straight from the database with no cache
/// and no render in the way. The oracle every coherence check compares a
/// served body against.
async fn row_views(id: i64) -> Result<i64, Box<dyn Error>> {
    Ok(Post::find(id).await?.map_or(-1, |post| post.views))
}

// -------------------------------------------------------------------------
// Workload 1: the middleware hot hit
// -------------------------------------------------------------------------

/// The `C64` route served through the whole middleware: a 64 KiB body over
/// twelve ORM reads, under lease coherence.
///
/// The first request renders and publishes; the second is the first hot hit
/// and is what grants the validation lease, so the measured requests start
/// from the third.
///
/// The statement count is taken from one armed request, and read only after
/// the client has the whole body *and* after `hot_serves_for_test` has
/// moved, so neither the response write nor the hot-path bookkeeping can
/// still be outstanding when it is read. The residual gap is a statement
/// this request causes that is issued after both of those - a background
/// task the middleware detached, say - which no counter read at any single
/// point can see; nothing on the hot path does that today, and the storm's
/// own armed hit would show a statement that appeared later as a count of
/// two rather than one.
async fn run_c64_middleware(
    harness: &Harness,
    timed: bool,
) -> Result<C64Middleware, Box<dyn Error>> {
    let ids = create_posts(C64_DEPENDENCY_ROWS).await?;
    require(
        ids.first() == Some(&1) && ids.last() == Some(&(C64_DEPENDENCY_ROWS as i64)),
        "the C64 route reads a contiguous run of row ids from 1, which a fresh database gives it",
    )?;
    let key = RenderCache::key_for_route_for_test(C64_ROUTE, &[("id", "1")], None);

    let first = dispatch_get(harness, C64_PATH, &[]).await;
    expect(first.status.as_u16(), 200, "the first request's status")?;
    expect(first.body.len(), C64_BODY_BYTES, "the C64 body's length")?;
    expect(
        counting_route::renders(),
        1,
        "renders the first request through the measured route caused",
    )?;
    require(
        RenderCache::l0_hot_for_test(&key),
        "the first request must publish a hot entry for the measured requests to hit",
    )?;
    let published = RenderCache::inspect(&key)
        .await?
        .ok_or_else(|| io::Error::other("the first request must publish an entry to inspect"))?;
    let dependencies = published.observations;
    require(
        dependencies >= C64_DEPENDENCY_ROWS,
        "the C64 entry must observe at least one dependency identity per row its handler read",
    )?;
    expect(
        published.body_bytes,
        C64_BODY_BYTES,
        "the stored body length",
    )?;
    dispatch_get(harness, C64_PATH, &[]).await;

    // The condition the microseconds below are only meaningful beside: a
    // lease-mode hot hit reaches the database zero times. A change that
    // read authority per request - dropping the epoch lease, or checking
    // coherence against the ledger on a hit - would make this 1 and fail
    // here rather than quietly costing a round trip per request.
    statements::reset();
    let hot_before = RenderCache::hot_serves_for_test();
    let armed = dispatch_get(harness, C64_PATH, &[]).await;
    expect(armed.status.as_u16(), 200, "the armed hit's status")?;
    expect(
        armed.body.len(),
        C64_BODY_BYTES,
        "the armed hit's body length",
    )?;
    expect(
        RenderCache::hot_serves_for_test(),
        hot_before + 1,
        "hot serves after one armed request",
    )?;
    let statements_per_hit = statements::count();
    expect(
        statements_per_hit,
        0,
        "statements a lease-mode hot hit issues",
    )?;

    let server_timings: ServerTimingLog = Arc::new(Mutex::new(Vec::new()));
    let warmup = scaled(WARMUP, timed);
    for _ in 0..warmup {
        dispatch_get_timed(harness, C64_PATH, &server_timings).await;
    }
    server_timings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();

    let samples = scaled(SAMPLES, timed);
    // A shared reference, which is `Copy`, so each call of the `FnMut`
    // below hands its future a copy rather than moving the handle into the
    // first one.
    let timings = &server_timings;
    let round_trip = measure(samples, elapsed_microseconds, || async move {
        let response = dispatch_get_timed(harness, C64_PATH, timings).await;
        expect(response.status.as_u16(), 200, "a measured hit's status")?;
        expect(
            response.body.len(),
            C64_BODY_BYTES,
            "a measured hit's body length",
        )
    })
    .await?;
    let server: Vec<f64> = server_timings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|elapsed| elapsed.as_secs_f64() * 1_000_000.0)
        .collect();
    expect(
        server.len(),
        samples,
        "server-side durations recorded across the measured pass",
    )?;

    // Nothing rendered while the measurement ran, so every sample above is
    // a hit rather than an occasional rebuild averaged in.
    expect(
        counting_route::renders(),
        1,
        "renders across the whole measured pass",
    )?;

    let server = Timing::of(server, timed);
    let round_trip = Timing::of(round_trip, timed);
    println!(
        "c64_middleware: server p50={:.3}us p95={:.3}us round_trip p50={:.3}us p95={:.3}us \
         body_bytes={C64_BODY_BYTES} dependencies={dependencies} \
         statements_per_hit={statements_per_hit}",
        server.p50, server.p95, round_trip.p50, round_trip.p95
    );
    Ok(C64Middleware {
        body_bytes: C64_BODY_BYTES,
        dependencies,
        transport: C64_TRANSPORT,
        warmup,
        samples,
        p50_microseconds: server.p50,
        p95_microseconds: server.p95,
        round_trip_p50_microseconds: round_trip.p50,
        round_trip_p95_microseconds: round_trip.p95,
        statements_per_hit,
    })
}

// -------------------------------------------------------------------------
// Workload 2: the batched coherence reread
// -------------------------------------------------------------------------

/// The twelve dependency digests a reread carries.
fn reread_digests() -> Vec<[u8; 32]> {
    (0..IDENTITIES)
        .map(|index| DependencyIdentity::record("posts", index.to_string().as_bytes()).digest())
        .collect()
}

/// `current_with_epoch` over twelve digests: the one statement a hit in
/// authority mode pays.
///
/// The statement count is the condition. `current_with_epoch` has a
/// default implementation that reads the generations and then the epoch -
/// two round trips - which the SQL ledger overrides with one `UNION ALL`.
/// A change that dropped that override, or that read the epoch separately
/// again, would make this 2 and fail here.
async fn run_generation_reread(timed: bool) -> Result<GenerationReread, Box<dyn Error>> {
    let ledger = ledger();
    let digests = reread_digests();

    let warmup = scaled(WARMUP, timed);
    for _ in 0..warmup {
        ledger.current_with_epoch(&digests).await?;
    }

    statements::reset();
    ledger.current_with_epoch(&digests).await?;
    let statements_per_reread = statements::count();
    expect(
        statements_per_reread,
        1,
        "statements one batched coherence reread issues",
    )?;

    let samples = scaled(SAMPLES, timed);
    // Shared references for the same reason `run_c64_middleware` takes
    // them: they are `Copy`, so the closure stays callable.
    let (ledger, digests) = (&ledger, &digests);
    let timing = Timing::of(
        measure(samples, elapsed_milliseconds, || async move {
            ledger.current_with_epoch(digests).await?;
            Ok(())
        })
        .await?,
        timed,
    );

    if timed && timing.p95 > REREAD_CAP_MILLISECONDS {
        return Err(io::Error::other(format!(
            "the generation reread reached p95 {:.3} ms, above the {REREAD_CAP_MILLISECONDS:.1} ms ceiling",
            timing.p95
        ))
        .into());
    }

    println!(
        "generation_reread: p50={:.4}ms p95={:.4}ms statements_per_reread={statements_per_reread}",
        timing.p50, timing.p95
    );
    Ok(GenerationReread {
        keys: IDENTITIES,
        warmup,
        samples,
        p50_milliseconds: timing.p50,
        p95_milliseconds: timing.p95,
        cap_milliseconds: REREAD_CAP_MILLISECONDS,
        statements_per_reread,
    })
}

// -------------------------------------------------------------------------
// Workload 3: the invalidation storm
// -------------------------------------------------------------------------

/// The sixty-four cached paths, spread over the twelve rows the storm
/// writes to. `page` is a declared query parameter on this route's policy,
/// so `?page=n` is part of the lookup key; the handler ignores it.
fn storm_paths(post_ids: &[i64]) -> Vec<String> {
    (0..STORM_KEYS)
        .map(|index| {
            let id = post_ids[index % IDENTITIES];
            let page = index / IDENTITIES + 1;
            format!("{}/{id}?page={page}", STORM_ROUTE.trim_end_matches("/{id}"))
        })
        .collect()
}

/// Advances one row's record generation, and the table generation with it,
/// through the ORM inside its own transaction - the shape any application
/// write takes. `views` is the column the storm route renders, so a write
/// here is one a served body can be checked against.
async fn advance_storm_row(id: i64) -> Result<(), Box<dyn Error>> {
    DB::transaction(move |_tx| {
        Box::pin(async move {
            let mut post = Post::find(id).await?.expect("a storm row this run created");
            post.views += 1;
            post.save().await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await?;
    Ok(())
}

/// Measures whether a write to one row invalidates a key that never read
/// that row.
///
/// Iteration 006, definition-of-done item 5 narrowed `Model::find` on a
/// hit: it now observes the row's own record and the table's unkeyed-write
/// identity, not the table itself (`framework/src/eloquent/model.rs`), so
/// a row-level write elsewhere in the table - the shape `advance_storm_row`
/// takes - no longer reaches a key that read a different row. This
/// therefore returns `false` since that change landed; the result file
/// says so under `fanout_is_table_wide`. The burst shape below still
/// invalidates every key every burst regardless: `writes_per_burst` is at
/// least [`IDENTITIES`], so every row's own record is written inside each
/// burst, which record-level invalidation catches on its own. A change
/// that made this return `true` again, or that shrank a burst below one
/// write per identity, is what would need the shape re-checked.
async fn measure_write_fanout(harness: &Harness, post_ids: &[i64]) -> Result<bool, Box<dyn Error>> {
    let untouched = format!(
        "{}/{}?page=1",
        STORM_ROUTE.trim_end_matches("/{id}"),
        post_ids[0]
    );
    dispatch_get(harness, &untouched, &[]).await;
    let published = counting_route::renders();
    dispatch_get(harness, &untouched, &[]).await;
    expect(
        counting_route::renders(),
        published,
        "renders a second request to an unwritten key causes",
    )?;

    advance_storm_row(post_ids[1]).await?;
    let before = counting_route::renders();
    dispatch_get(harness, &untouched, &[]).await;
    Ok(counting_route::renders() == before + 1)
}

/// The fraction of this workload's keys that one row-level write to one
/// post invalidates.
///
/// Measured the way [`measure_write_fanout`] measures, but over every key
/// rather than one: warm the whole set, prove it quiet, write one row, and
/// count how many keys rebuild on the next pass. With the point-read rule
/// in place the answer is the number of keys that read the written row over
/// the number of keys, so this workload's twelve record identities behind
/// sixty-four keys give roughly one in eleven, not the one it used to be.
///
/// Leaves every key warm except the ones the write invalidated, which is
/// what the storm's own first sweep expects, since its first burst writes
/// every row.
async fn measure_point_read_invalidation_ratio(
    harness: &Harness,
    paths: &[String],
    post_ids: &[i64],
) -> Result<f64, Box<dyn Error>> {
    for path in paths {
        dispatch_get(harness, path, &[]).await;
    }
    let warm = counting_route::renders();
    for path in paths {
        dispatch_get(harness, path, &[]).await;
    }
    expect(
        counting_route::renders(),
        warm,
        "renders a repeat pass over a warm key set causes",
    )?;

    advance_storm_row(post_ids[0]).await?;
    let before = counting_route::renders();
    for path in paths {
        dispatch_get(harness, path, &[]).await;
    }
    let invalidated = counting_route::renders() - before;

    // Through `f64::from(u32)`, which is lossless, so neither side needs a
    // cast or a lint suppression.
    let invalidated = f64::from(u32::try_from(invalidated)?);
    let total = f64::from(u32::try_from(paths.len())?);
    Ok(invalidated / total)
}

/// Requests every key once, returning each response's body and the
/// microseconds it took.
///
/// The caller says which of the two sweeps this is, and the sweep proves
/// it: after a burst of writes every key must rebuild, and immediately
/// afterwards every key must be served from its hot entry. Counting both
/// against `renders` and `hot_serves` is what makes "the storm did not
/// cause a render storm" a measurement rather than a hope.
async fn storm_sweep(
    harness: &Harness,
    paths: &[String],
    expect_hits: bool,
) -> Result<(Vec<Bytes>, Vec<f64>), Box<dyn Error>> {
    let renders_before = counting_route::renders();
    let hot_before = RenderCache::hot_serves_for_test();
    let mut bodies = Vec::with_capacity(paths.len());
    let mut latencies = Vec::with_capacity(paths.len());

    for path in paths {
        let started = Instant::now();
        let response = dispatch_get(harness, path, &[]).await;
        latencies.push(elapsed_microseconds(started));
        expect(response.status.as_u16(), 200, "a swept key's status")?;
        bodies.push(response.body);
    }

    let rendered = counting_route::renders() - renders_before;
    let hot = RenderCache::hot_serves_for_test() - hot_before;
    let keys = paths.len() as u64;
    if expect_hits {
        expect(
            hot,
            keys,
            "keys served from a hot entry in the second sweep",
        )?;
        expect(rendered, 0, "renders during the second sweep")?;
    } else {
        expect(
            rendered,
            keys,
            "keys rebuilt in the sweep after a write burst",
        )?;
    }
    Ok((bodies, latencies))
}

/// Twenty bursts of fifty writes, each burst followed by a sweep that must
/// rebuild every key and a sweep that must hit every key.
///
/// The sweeps are quiescent by necessity, not by preference: every write
/// invalidates every key (see [`measure_write_fanout`]), so no key can be a
/// hit while a write is in flight, and the hit latency this reports is
/// named `quiescent_hit_p95_microseconds` for that reason. What the shape
/// does measure is the one thing spec 18 asks a storm to prove: one
/// thousand writes across sixty-four keys cost rebuilds in the low
/// thousands, not the 64,000 an invalidate-and-rebuild-on-every-request
/// cache would pay. The aggregate is checked against bounds derived from
/// the shape rather than against a constant; the exact per-sweep contract
/// lives in [`storm_sweep`], where a key that failed to rebuild or failed
/// to hit fails immediately.
async fn run_invalidation_storm(
    harness: &Harness,
    timed: bool,
) -> Result<InvalidationStorm, Box<dyn Error>> {
    let post_ids = create_posts(IDENTITIES).await?;
    let paths = storm_paths(&post_ids);
    let every_write_invalidates_every_key = measure_write_fanout(harness, &post_ids).await?;
    let point_read_invalidation_ratio =
        measure_point_read_invalidation_ratio(harness, &paths, &post_ids).await?;

    let bursts = scaled(STORM_BURSTS, timed);
    let writes = if timed {
        STORM_WRITES
    } else {
        bursts * IDENTITIES
    };
    let writes_per_burst = writes / bursts;

    let renders_before = counting_route::renders();
    let hot_before = RenderCache::hot_serves_for_test();
    let mut hit_latencies = Vec::with_capacity(bursts * STORM_KEYS);
    let mut written = 0_usize;

    for burst in 0..bursts {
        let burst_writes = if burst + 1 == bursts {
            writes - written
        } else {
            writes_per_burst
        };
        for write in 0..burst_writes {
            advance_storm_row(post_ids[(written + write) % IDENTITIES]).await?;
        }
        written += burst_writes;

        let (rebuilt, _) = storm_sweep(harness, &paths, false).await?;
        let (served, latencies) = storm_sweep(harness, &paths, true).await?;
        require(
            rebuilt == served,
            "a hit after a rebuild must serve exactly the bytes that rebuild published",
        )?;
        hit_latencies.extend(latencies);
    }
    expect(written, writes, "writes the storm committed")?;

    let rebuilds = counting_route::renders() - renders_before;
    let hits = RenderCache::hot_serves_for_test() - hot_before;
    let floor = (bursts * STORM_KEYS) as u64;
    let ceiling = floor * SWEEPS_PER_BURST as u64;
    require(
        rebuilds >= floor && rebuilds <= ceiling,
        "the storm's rebuilds must lie between one per key per burst and one per key per sweep",
    )?;
    require(
        rebuilds >= writes as u64,
        "the shape must keep the storm meaningful: at least one rebuild per write on average",
    )?;
    // Bounded by the same shape as `rebuilds`, and for the same reason: an
    // aggregate pinned to a constant restates the loop bounds instead of
    // measuring the cache. The exact discrimination - every key of the
    // second sweep of every burst served hot, and none of the first - is
    // the per-sweep `expect` in `storm_sweep`, which fails on the sweep
    // that broke it rather than on the total.
    require(
        hits >= floor && hits <= ceiling,
        "the storm's hot serves must lie between one per key per burst and one per key per sweep",
    )?;

    // One armed hit, after the storm: an authority-mode hit is the single
    // batched reread and nothing else. Read after the body is collected and
    // after the hot-serve counter moved, for the reason
    // `run_c64_middleware` gives.
    statements::reset();
    let hot_armed = RenderCache::hot_serves_for_test();
    let armed = dispatch_get(harness, &paths[0], &[]).await;
    expect(armed.status.as_u16(), 200, "the armed hit's status")?;
    expect(
        RenderCache::hot_serves_for_test(),
        hot_armed + 1,
        "hot serves after the armed hit",
    )?;
    let statements_per_hit = statements::count();
    expect(
        statements_per_hit,
        1,
        "statements an authority-mode hot hit issues",
    )?;

    // The storm's last word, checked against the database rather than
    // against another sweep: one more write to every row, then every key
    // must rebuild, and the body it serves must name its own row and carry
    // the `views` that row now holds. The id is checked beside the `views`
    // because the twelve rows share a route: a body served for the wrong
    // row would pass a `views`-only comparison whenever the two rows have
    // drifted to the same count.
    for id in &post_ids {
        advance_storm_row(*id).await?;
    }
    let (final_bodies, _) = storm_sweep(harness, &paths, false).await?;
    let mut final_bodies_coherent = true;
    for (index, body) in final_bodies.iter().enumerate() {
        let id = post_ids[index % IDENTITIES];
        let (served_id, served_views) = storm_body_row(body).ok_or_else(|| {
            io::Error::other("every storm body must name its row and carry that row's `views`")
        })?;
        if served_id != id || served_views != row_views(id).await? {
            final_bodies_coherent = false;
        }
    }
    require(
        final_bodies_coherent,
        "every key must serve its own row's id and the `views` that row holds after the storm's \
         last write",
    )?;

    let timing = Timing::of(hit_latencies, timed);
    let rebuilds_per_write = rebuilds as f64 / writes as f64;
    println!(
        "invalidation_storm: hits={hits} rebuilds={rebuilds} per_write={rebuilds_per_write:.4} \
         quiescent_hit_p95={:.3}us statements_per_hit={statements_per_hit} \
         fanout_is_table_wide={every_write_invalidates_every_key} \
         point_read_ratio={point_read_invalidation_ratio:.4}",
        timing.p95
    );
    Ok(InvalidationStorm {
        keys: STORM_KEYS,
        identities: IDENTITIES,
        writes,
        bursts,
        writes_per_burst,
        sweeps_per_burst: SWEEPS_PER_BURST,
        every_write_invalidates_every_key,
        point_read_invalidation_ratio,
        hits,
        rebuilds,
        rebuilds_per_write,
        statements_per_hit,
        quiescent_hit_p95_microseconds: timing.p95,
        final_bodies_coherent,
    })
}

// -------------------------------------------------------------------------
// Workload 4: two nodes over one backend
// -------------------------------------------------------------------------

/// A lease store whose view of store time this bench can move forward.
///
/// Both adapters carry the same doc-hidden seam, under the same name, as an
/// inherent method. `framework/tests/render_cache/tiers/mod.rs` defines an
/// equivalent trait for the same reason; it lives in a test module this
/// bench does not include, and the method is renamed here so an inherent
/// call and a trait call can never be confused for one another.
trait OffsetLeaseStore: LeaseStore {
    fn set_time_offset(&self, offset_ms: u64);
}

impl OffsetLeaseStore for SqlLeaseStore {
    fn set_time_offset(&self, offset_ms: u64) {
        self.set_time_offset_for_test(offset_ms);
    }
}

impl OffsetLeaseStore for RedisLeaseStore {
    fn set_time_offset(&self, offset_ms: u64) {
        self.set_time_offset_for_test(offset_ms);
    }
}

/// Every handle one `multi_node` run needs.
///
/// Built by the caller, because only the caller knows which tier's
/// constructors to call and which of them are asynchronous. Two coordinator
/// handles over one backend are two nodes, for the reason
/// `framework/tests/render_cache/tiers/mod.rs` gives: the RenderCache
/// runtime is a process singleton, so a second runtime is not something one
/// process can have, and two adapter handles over one backend are what a
/// second node actually is to the backend.
struct Nodes<L: LeaseStore, R: RenderStore> {
    fan_in: Vec<Arc<FencedLeaseCoordinator<L>>>,
    fan_in_stores: Vec<Arc<R>>,
    dying: FencedLeaseCoordinator<L>,
    dying_store: Arc<L>,
    taking: FencedLeaseCoordinator<L>,
    taking_store: Arc<L>,
    entries: R,
}

/// A coordinator handle over `store`.
fn node<L: LeaseStore>(store: Arc<L>, lease_ms: u64) -> FencedLeaseCoordinator<L> {
    FencedLeaseCoordinator::new(
        store,
        LocalCoordinatorLimits {
            lease_ms,
            max_waiters: MAX_WAITERS,
        },
    )
}

/// The database tier's handles.
fn sql_nodes() -> Nodes<SqlLeaseStore, SqlRenderStore> {
    let dying_store = Arc::new(SqlLeaseStore::new());
    let taking_store = Arc::new(SqlLeaseStore::new());
    Nodes {
        fan_in: (0..NODES)
            .map(|_| Arc::new(node(Arc::new(SqlLeaseStore::new()), FAN_IN_LEASE_MS)))
            .collect(),
        fan_in_stores: (0..NODES)
            .map(|_| Arc::new(SqlRenderStore::new(MAX_STORE_BYTES)))
            .collect(),
        dying: node(Arc::clone(&dying_store), TAKEOVER_LEASE_MS),
        dying_store,
        taking: node(Arc::clone(&taking_store), TAKEOVER_LEASE_MS),
        taking_store,
        entries: SqlRenderStore::new(MAX_STORE_BYTES),
    }
}

/// The Redis tier's handles, over the namespace `config` names.
async fn redis_nodes(
    config: &RedisProviderConfig,
) -> Result<Nodes<RedisLeaseStore, RedisRenderStore>, Box<dyn Error>> {
    let dying_store = Arc::new(RedisLeaseStore::connect(config).await?);
    let taking_store = Arc::new(RedisLeaseStore::connect(config).await?);
    let mut fan_in = Vec::with_capacity(NODES);
    let mut fan_in_stores = Vec::with_capacity(NODES);
    for _ in 0..NODES {
        fan_in.push(Arc::new(node(
            Arc::new(RedisLeaseStore::connect(config).await?),
            FAN_IN_LEASE_MS,
        )));
        fan_in_stores.push(Arc::new(
            RedisRenderStore::connect(config, MAX_STORE_BYTES).await?,
        ));
    }
    Ok(Nodes {
        fan_in,
        fan_in_stores,
        dying: node(Arc::clone(&dying_store), TAKEOVER_LEASE_MS),
        dying_store,
        taking: node(Arc::clone(&taking_store), TAKEOVER_LEASE_MS),
        taking_store,
        entries: RedisRenderStore::connect(config, MAX_STORE_BYTES).await?,
    })
}

/// Sixty-four cold requests for one key, split across two nodes.
///
/// Admission is collected for every request before any leader publishes,
/// which is what makes the count exact rather than a race: while nothing
/// has published or released, exactly one node holds the distributed
/// lease, so exactly one request can lead and exactly one publication can
/// follow. A change that let a second node publish under the same lease -
/// a lease that is not fenced, or a store that accepts an equal fence -
/// fails on `publications`.
///
/// Each request's sample is the whole time it took that request to learn
/// what to serve: the leader's runs to the end of its publication, a
/// bypassing node's ends at its admission because it renders on its own
/// from there, and a waiter's ends when the leader released it. Collecting
/// admissions first costs a waiter only the microseconds the other
/// admissions take, so what a waiter's sample is dominated by is the
/// publication it was actually waiting for.
async fn fan_in<L, R>(
    nodes: &[Arc<FencedLeaseCoordinator<L>>],
    stores: &[Arc<R>],
    key: RenderKey,
    bytes: Bytes,
    requests: usize,
) -> Result<(usize, usize, Vec<f64>), Box<dyn Error>>
where
    L: LeaseStore + 'static,
    R: RenderStore,
{
    let mut admitting = tokio::task::JoinSet::new();
    for request in 0..requests {
        let coordinator = Arc::clone(&nodes[request % nodes.len()]);
        let requested = key.clone();
        admitting.spawn(async move {
            let started = Instant::now();
            let admission = coordinator.admit(&requested, NODE_EPOCH, 0).await;
            (request, started, elapsed_microseconds(started), admission)
        });
    }

    let mut latencies = Vec::with_capacity(requests);
    let mut leads = Vec::new();
    let mut waits = Vec::new();
    let mut duplicate_renders = 0_usize;
    let mut admitted = 0_usize;
    while let Some(joined) = admitting.join_next().await {
        let (request, started, microseconds, admission) = joined?;
        admitted += 1;
        match admission? {
            RebuildAdmission::Lead(lease) => leads.push((request, started, lease)),
            RebuildAdmission::Bypass => {
                duplicate_renders += 1;
                latencies.push(microseconds);
            }
            RebuildAdmission::Wait(waiter) => waits.push((started, waiter)),
        }
    }
    expect(
        leads.len(),
        1,
        "requests admitted to lead one key across both nodes",
    )?;
    expect(
        admitted,
        requests,
        "requests that reached an admission decision",
    )?;

    let mut publications = 0_usize;
    for (request, started, lease) in leads {
        let coordinator = Arc::clone(&nodes[request % nodes.len()]);
        let store = Arc::clone(&stores[request % stores.len()]);
        let fence = coordinator.publish_token(&lease, 0).await?;
        let outcome = store
            .publish(
                &key,
                bytes.clone(),
                fence,
                PUBLISHED_AT_MS,
                PUBLICATION_RETENTION_MS,
            )
            .await?;
        if outcome == PublishOutcome::Published {
            publications += 1;
        }
        coordinator.release(*lease).await?;
        latencies.push(elapsed_microseconds(started));
    }
    for (started, waiter) in waits {
        waiter.wait().await;
        latencies.push(elapsed_microseconds(started));
    }
    expect(
        latencies.len(),
        requests,
        "requests that reached a servable outcome",
    )?;

    // Every node reads the one entry, whichever handle published it.
    for store in stores {
        let stored = store
            .get(&key)
            .await?
            .ok_or_else(|| io::Error::other("both nodes must read the published entry"))?;
        require(
            stored.bytes == bytes,
            "the bytes every node reads are the leader's",
        )?;
    }
    Ok((publications, duplicate_renders, latencies))
}

/// One takeover: a leader whose lease elapsed by store time is taken over,
/// the takeover publishes, and the fenced leader mints nothing.
///
/// Store time is moved rather than waited for, so the round costs a few
/// statements instead of a lease lifetime, and each round moves it one step
/// further than the round before it so no round can reach an earlier
/// round's lease. The fenced leader's refusal is the correctness half: a
/// `publish_token` that answered a node's own clock, or a lease that never
/// fenced, would let the dead leader publish over the takeover's bytes.
async fn takeover_round<L, R>(
    pattern: &str,
    nodes: &Nodes<L, R>,
    before_ms: u64,
    after_ms: u64,
) -> Result<f64, Box<dyn Error>>
where
    L: OffsetLeaseStore,
    R: RenderStore,
{
    let key = render_cache_tiers_support::key(pattern);
    nodes.dying_store.set_time_offset(before_ms);
    nodes.taking_store.set_time_offset(before_ms);

    let RebuildAdmission::Lead(dead) = nodes.dying.admit(&key, NODE_EPOCH, before_ms).await? else {
        return Err(io::Error::other("the first node must lead an unheld key").into());
    };
    nodes.dying_store.set_time_offset(after_ms);
    nodes.taking_store.set_time_offset(after_ms);

    let started = Instant::now();
    let RebuildAdmission::Lead(taken) = nodes.taking.admit(&key, NODE_EPOCH, after_ms).await?
    else {
        return Err(io::Error::other("an elapsed lease must be taken over").into());
    };
    let fence = nodes.taking.publish_token(&taken, after_ms).await?;
    let outcome = nodes
        .entries
        .publish(
            &key,
            render_cache_tiers_support::encoded_entry(pattern),
            fence,
            PUBLISHED_AT_MS,
            PUBLICATION_RETENTION_MS,
        )
        .await?;
    let milliseconds = elapsed_milliseconds(started);
    expect(outcome, PublishOutcome::Published, "the takeover's publish")?;
    nodes.taking.release(*taken).await?;

    let refused = nodes
        .dying
        .publish_token(&dead, after_ms)
        .await
        .err()
        .map(|error| error.kind());
    expect(
        refused,
        Some(suprnova_live::render_cache::RenderCacheErrorKind::LeaseFenced),
        "what a fenced leader's publish token is refused with",
    )?;
    nodes.dying.release(*dead).await?;
    Ok(milliseconds)
}

/// The fan-in and the takeover, over whatever backend `nodes` addresses.
///
/// `namespace` keeps one tier's keys away from another's, so a run that
/// measures both leaves neither reading the other's entries.
async fn run_multi_node<L, R>(
    nodes: &Nodes<L, R>,
    namespace: &str,
    timed: bool,
) -> Result<MultiNode, Box<dyn Error>>
where
    L: OffsetLeaseStore + 'static,
    R: RenderStore,
{
    // A warm round first, over its own key, so the measured round is not
    // paying for the pool's first statement against these tables.
    let warm = format!("{namespace}/fan-in-warm");
    fan_in(
        &nodes.fan_in,
        &nodes.fan_in_stores,
        render_cache_tiers_support::key(&warm),
        render_cache_tiers_support::encoded_entry(&warm),
        NODES,
    )
    .await?;

    let measured = format!("{namespace}/fan-in");
    let (publications, duplicate_renders, latencies) = fan_in(
        &nodes.fan_in,
        &nodes.fan_in_stores,
        render_cache_tiers_support::key(&measured),
        render_cache_tiers_support::encoded_entry(&measured),
        CONCURRENT_REQUESTS,
    )
    .await?;
    expect(
        publications,
        1,
        "publications sixty-four concurrent cold requests across two nodes make",
    )?;
    let fan_in_timing = Timing::from_samples(latencies);

    let rounds = scaled(TAKEOVER_ROUNDS, timed);
    let mut takeovers = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let step = round as u64 * TAKEOVER_OFFSET_STEP_MS;
        takeovers.push(
            takeover_round(
                &format!("{namespace}/takeover-{round}"),
                nodes,
                step,
                step + TAKEOVER_OFFSET_STEP_MS,
            )
            .await?,
        );
    }
    let takeover_timing = Timing::from_samples(takeovers);

    println!(
        "multi_node[{namespace}]: publications={publications} \
         duplicate_renders={duplicate_renders} fan_in_p95={:.3}us takeover_p95={:.4}ms",
        fan_in_timing.p95, takeover_timing.p95
    );
    Ok(MultiNode {
        nodes: NODES,
        concurrent_requests: CONCURRENT_REQUESTS,
        publications,
        duplicate_renders,
        fan_in_p95_microseconds: fan_in_timing.p95,
        takeover_p95_milliseconds: takeover_timing.p95,
    })
}

// -------------------------------------------------------------------------
// Runs
// -------------------------------------------------------------------------

/// Every workload, over the middleware harness's own SQLite file and the
/// tier tables beside it.
async fn sqlite_run(timed: bool) -> Result<DatabaseRun, Box<dyn Error>> {
    let c64_middleware = {
        let harness = boot_with_render_cache().await;
        let measured = run_c64_middleware(&harness, timed).await?;
        drop(harness);
        measured
    };
    let generation_reread = {
        let harness = boot_with_render_cache().await;
        let measured = run_generation_reread(timed).await?;
        drop(harness);
        measured
    };
    let invalidation_storm = {
        let harness = boot_with_render_cache().await;
        let measured = run_invalidation_storm(&harness, timed).await?;
        drop(harness);
        measured
    };
    let multi_node = {
        let database = render_cache_tiers_support::boot().await;
        let measured = run_multi_node(&sql_nodes(), "/workloads-sqlite", timed).await?;
        drop(database);
        measured
    };

    Ok(DatabaseRun {
        database: "sqlite",
        accelerator: "none",
        c64_middleware: Some(c64_middleware),
        generation_reread: Some(generation_reread),
        invalidation_storm: Some(invalidation_storm),
        multi_node: Some(multi_node),
    })
}

/// The reread and the multi-node workload again, against a live
/// PostgreSQL server. The middleware harness's own two workloads stay on
/// SQLite: they measure a served request rather than a backend, and the
/// harness that serves them owns its database.
///
/// The statement observer is installed on the connection before anything
/// clones it, which is the only moment SeaORM accepts one - so this
/// connects the pool itself rather than taking one from the tier helper.
async fn postgres_run(url: &str, timed: bool) -> Result<DatabaseRun, Box<dyn Error>> {
    let generation_reread = {
        let config = suprnova::database::DatabaseConfig::builder()
            .url(url.to_owned())
            .max_connections(4)
            .min_connections(1)
            .logging(false)
            .build();
        let mut conn = suprnova::database::DbConnection::connect(&config).await?;
        require(
            statements::install(&mut conn),
            "a freshly connected pool must accept the statement observer",
        )?;
        let harness = boot_with_render_cache_on_live_server_for_test(conn).await;
        let measured = run_generation_reread(timed).await?;
        drop(harness);
        measured
    };

    let multi_node = {
        let conn = render_cache_tiers_support::try_connect_live(url)
            .await
            .ok_or_else(|| {
                io::Error::other("PG_TEST_URL is set but the server is not reachable")
            })?;
        let guard = render_cache_tiers_support::reset_and_migrate(conn).await;
        let measured = run_multi_node(&sql_nodes(), "/workloads-postgres", timed).await?;
        drop(guard);
        measured
    };

    Ok(DatabaseRun {
        database: "postgres",
        accelerator: "none",
        c64_middleware: None,
        generation_reread: Some(generation_reread),
        invalidation_storm: None,
        multi_node: Some(multi_node),
    })
}

/// The multi-node workload over the Redis lease store and Redis render
/// store.
///
/// `database` is `"none"` and means it: nothing in this run reads or writes
/// a database. Both stores are Redis, under a namespace of this run's own
/// that the boot helper's guard removes afterwards.
async fn redis_run(timed: bool) -> Result<DatabaseRun, Box<dyn Error>> {
    let (config, _connection, cleanup) = render_cache_tiers_support::boot_redis().await;
    let nodes = redis_nodes(&config).await?;
    let multi_node = run_multi_node(&nodes, "/workloads-redis", timed).await?;
    drop(cleanup);

    Ok(DatabaseRun {
        database: "none",
        accelerator: "redis",
        c64_middleware: None,
        generation_reread: None,
        invalidation_storm: None,
        multi_node: Some(multi_node),
    })
}

fn main() {
    if let Err(error) = run() {
        eprintln!("render cache workloads failed: {error}");
        std::process::exit(1);
    }
}

/// The environment, every run, and the result file. A debug build runs
/// every correctness assertion at reduced counts and skips timing, as
/// `snapshot_budget` does; only a release run writes a result.
fn run() -> Result<(), Box<dyn Error>> {
    let mut environment = bench_environment::collect();
    let postgres = std::env::var("PG_TEST_URL")
        .ok()
        .filter(|url| !url.is_empty());
    let redis = std::env::var("REDIS_TEST_URL")
        .ok()
        .filter(|url| !url.is_empty());
    environment.database = if postgres.is_some() {
        "sqlite_and_postgres"
    } else {
        "sqlite"
    };
    environment.provider_versions = BTreeMap::from([
        ("generation_ledger", "sql_v1"),
        ("lease_store", "sql_fenced_v1"),
        ("render_store", "sql_l1_v1"),
    ]);
    if redis.is_some() {
        environment
            .provider_versions
            .insert("accelerator_lease_store", "redis_fenced_v1");
        environment
            .provider_versions
            .insert("accelerator_render_store", "redis_l1_v1");
    }
    assert_required_environment(&environment)?;
    let timed = !cfg!(debug_assertions);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut runs = vec![runtime.block_on(sqlite_run(timed))?];
    if let Some(url) = postgres {
        runs.push(runtime.block_on(postgres_run(&url, timed))?);
    }
    if redis.is_some() {
        runs.push(runtime.block_on(redis_run(timed))?);
    }

    if !timed {
        println!("render cache workloads debug contract checks only; release timing skipped");
        return Ok(());
    }

    let result = WorkloadsResult {
        schema_version: 1,
        profile: "release",
        measured_at_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        environment,
        runs,
    };
    let path = result_path();
    write_result(&result, &path)?;
    println!(
        "render cache workloads written to {} environment={} runs={}",
        path.display(),
        result.environment.classification,
        result.runs.len()
    );
    Ok(())
}
