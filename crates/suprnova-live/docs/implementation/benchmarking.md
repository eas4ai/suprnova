# Live kernel benchmark reproduction

All checked results are versioned evidence, not marketing claims. The integrated
crate's current results are `local_exploratory`; validated S1 evidence remains
the qualification boundary for a release or public performance statement. None
of these tools run in `scripts/gate.sh`: the gate verifies correctness and
security, and budgets run on demand so a person can read the numbers.

## Snapshot-processing benchmark

The iteration 001 benchmark measures the complete trusted snapshot pipeline for
the named `A8/16` workload: verify, hydrate, deterministic dehydrate,
canonicalize, and sign. Its state is exactly 8 KiB, its response-size check uses
16 KiB of HTML, and component hooks, rendering, domain I/O, and providers are
outside the timed region.

Run a local exploratory measurement from the integrated crate root:

```sh
rtk env CARGO_INCREMENTAL=0 scripts/run-snapshot-budget.sh
```

The runner warms the pipeline for 500 iterations, then records 40 batches of
100 iterations. It writes p50 and p95 per-operation timings and the complete
environment record to `benchmarks/snapshot-budget-v1.json`. The checked result
must remain below 500 microseconds p95, 1 KiB of response control overhead, and
768 bytes of snapshot framework overhead.

The checked local result records 40 post-warmup samples with a 69.043
microsecond p95, 213 bytes of response control overhead, and 603 bytes of
snapshot framework overhead. Re-running the command replaces the requested
result path atomically and may produce different honest timing evidence.

## Action-framework benchmark

The `A8/16-action-framework` workload measures the server framework path over
an 8 KiB signed state and 16 KiB named response workload. It includes complete
v2 request parsing, instance verification, Tier 0 in-process revision claim,
hydration, prepared binding, registered no-op dispatch, and successor
classification. The application action body, external provider/domain I/O, and
Askama rendering are excluded so the benchmark isolates framework overhead.

Run it from the integrated crate root:

```sh
rtk env CARGO_INCREMENTAL=0 scripts/run-action-budget.sh
```

The runner records 40 post-warmup samples and enforces the architecture's
2-millisecond p95 cap. The checked local result is 122.309 microseconds p95.
Fixture identity, exact included/excluded stages, provider versions, compiler,
affinity, CPU, memory, kernel, and environment classification are written to
`benchmarks/action-budget-v1.json`.

## Macro expansion and compile budget

The fixed compile workspace contains 1-, 10-, and 100-component fixtures that
all resolve generated runtime paths through the integrated development final-
facade fixture.
The budget uses pinned nightly expansion for token/byte counts and isolated
MSRV `cargo check` work for each fixture:

```sh
rtk node scripts/check-expansion-budget.mjs
```

The checked local evidence records 1,762/15,622/154,222 expanded tokens,
10,174/92,884/919,984 expanded bytes, and 6,239/6,262/6,500 milliseconds of
isolated Rust/Cargo 1.94.0 check work for 1/10/100 components. The tool rejects
fixture drift, expansion size more than 10% above the checked baseline, and
token or byte growth above twelve times between consecutive fixtures. Isolated
check time is bounded only within one run: each larger fixture must finish
within twice the 1-component fixture's check time. Dependency compilation
dominates that check, so a per-component compile regression that matters shows
up as that ratio, and the same-run ratio cancels machine speed,
`CARGO_BUILD_JOBS`, and concurrent load, none of which a checked millisecond
baseline can. The recorded milliseconds and job setting are exploratory
context, are never compared against the checked baseline, and are never
presented as release-grade toolchain performance.
`tests/expansion_budget_rules.mjs` holds the rule contract and runs on demand
with `node tests/expansion_budget_rules.mjs`.

## RenderCache budget benchmark

The engine benchmark measures the two RenderCache workloads that need no
database, no router, and no socket: `C64`, a 64 KiB Complete representation
with 12 dependencies served as a Complete L0 hit, and `C64+4`, the same
public shell assembled with four 4 KiB stitch slots. The timed region is
engine work to a formed `http::Response<Bytes>`; the framework's conversion
into its own response type, the router, the middleware chain, and the socket
are all outside it, and the workload benchmark below is where those are
measured instead.

It is the only file in this crate that uses the `unsafe` keyword. The package
lint is `unsafe_code = "deny"` so that `benches/render_cache_budget.rs` can
carry one `#![allow(unsafe_code, reason = ..)]` for the counting global
allocator the allocation rows need; `src/lib.rs` keeps
`#![forbid(unsafe_code)]`, so no library, test, or example code can opt in.

Run both RenderCache benchmarks from the workspace root:

```sh
rtk env CARGO_INCREMENTAL=0 crates/suprnova-live/scripts/run-render-cache-budget.sh
```

The runner pins both benchmarks to `SUPRNOVA_LIVE_S1_CPUSET` (default `0-7`)
with `taskset` and finishes by running `tests/benchmark_contract.rs` over the
results. `SUPRNOVA_LIVE_SKIP_WORKLOADS=1` runs the engine benchmark alone;
`SUPRNOVA_LIVE_BENCH_RESULT` and `SUPRNOVA_LIVE_WORKLOADS_RESULT` redirect
the two result files, and a partial run must redirect both under the
gitignored `benchmarks/local/` or it overwrites the checked-in results with a
shorter file and then fails its own contract.

Before it measures anything the benchmark reads `/proc/self/status` and
refuses to run unless the process has exactly one thread, so the allocator
can never count another thread's work; there is no async runtime in the
measured path. Correctness guards run in every profile ahead of the
measurement - status, body length, entity tag, the literal `Cache-Control`
each fixture must serve, the served body's pointer and length, the 304 and
200 conditional answers, and, for the assembly, the exact assembled length,
one nonce in one hole, and each island marker once in slot order - so a run
that got fast by getting wrong fails instead of reporting.

The allocation pass runs 100 armed single requests per shape. The timing pass
is release-only and records 200 warmup iterations, then 40 samples of 50
iterations each, so one clock read covers work far larger than the clock's
own cost. Results go to `benchmarks/render-cache-budget-v1.json`.

| Row | Cap | Checked local result |
|---|---|---|
| `C64` p95 | 250 microseconds | 0.7557 |
| `C64` allocations | 4 | 3 fresh, 3 conditional, 4 seed-deadline |
| `C64+4` p95 | 2,000 microseconds | 34.215 |
| `C64+4` copy ratio | 2.0 | 1.0378 |

Every allocation figure is the maximum over its 100 passes, and all 100
recorded the same count. The checked result also records `body_shared`,
which the benchmark sets by comparing the served body's pointer and length
against the stored buffer's on every pass.

## RenderCache workload benchmark

The framework benchmark measures the four RenderCache workloads that need a
database, a router, or two nodes, and it contains no `unsafe`. Each workload
asserts the correctness condition its numbers are only meaningful beside, so
these are not timings alone: a lease-mode hot hit issues no statement, a
coherence reread is one batched statement, a write storm rebuilds each key
once per burst and leaves every key serving the generation the storm ended
on, and sixty-four concurrent cold requests across two nodes publish exactly
once.

`SUPRNOVA_LIVE_SKIP_WORKLOADS` must not be set for it to run, and a complete
run needs both `PG_TEST_URL` and `REDIS_TEST_URL`, because the checked-result
contract requires all three recorded profiles (SQLite, PostgreSQL, Redis).
Both servers must be disposable: the run drops and recreates every table and
flushes every key it uses. Every latency workload runs 200 requests before it
measures 200. Results go to `benchmarks/render-cache-workloads-v1.json`.

- `c64_middleware` drives the `C64` route through the real middleware in a
  test host. Its `p50`/`p95` pair is the server side, from the parsed request
  reaching the router to the response value existing, and excludes the
  connection, the response write, and the client's read; its round-trip pair
  is the whole loopback exchange around the same call. Checked: 8.716 and
  14.440 microseconds server side, 68.504 and 109.082 microseconds round
  trip, with zero statements per hit.
- `generation_reread` rereads 12 dependency keys and the epoch as one
  statement, against a 3 millisecond cap. Checked: 0.019 and 0.032
  milliseconds on SQLite, 0.092 and 0.236 milliseconds on PostgreSQL.
- `invalidation_storm` commits 1,000 writes in 20 bursts of 50 against 64
  cached keys. It records that a point read observes its table as well as its
  row, so every write invalidates every key, and reports the hit that follows
  a rebuild rather than a hit during the writes, which cannot exist. Checked:
  1,280 hits, 1,280 rebuilds, 1.28 rebuilds per write, one statement per hit,
  and a 165.048 microsecond quiescent hit p95.
- `multi_node` fans 64 concurrent cold requests for one key across two
  handles over one backend, through hand-driven coordinator calls rather than
  served requests. Checked on every tier: one publication, one bypass on the
  node that did not lead. Fan-in p95 is 166.501 microseconds on SQLite,
  9,201.986 on PostgreSQL, and 260.519 on Redis; takeover p95 is 1.3229,
  6.0536, and 0.2153 milliseconds.

Both RenderCache results are classified the same way every other budget tool
in this crate classifies its own. The checked-in files are
`local_exploratory` - a workstation, `powersave` governor, no dedicated-vCPU
attestation - and `SUPRNOVA_LIVE_REQUIRE_S1=1` turns a non-qualifying
environment into a refusal to measure rather than a labelled result. Nothing
here is S1 evidence or a public performance claim; see "Validated S1
evidence" below for what would be.

## Browser runtime benchmark

Artifact sizes are not a budget: `npm run build` prints the exact raw and
Brotli bytes of every artifact, and no artifact has a ceiling. The browser
benchmark below is the only browser-side budget tool.

Record an exploratory result with:

```sh
rtk npm --prefix browser run budget:browser
```

The harness uses pinned Chromium with 4x CPU throttling, five warmups, thirty
post-warmup samples, and thirty seconds of idle observation. D100 measures 100
island discovery/connection; M1K and M5K measure identity-preserving morphs of
1,000 and 5,000 nodes. It records p50/p95, bootstrap and idle work, observer
cardinality, artifact SHA-256/Brotli size, and retained bytes per island through
the browser heap instrumentation.

The current exploratory evidence is bound to artifact
`7e7f790ec2e6feeaf4f6bdd15655754b21657b604b26ec6d4136d4eebb401869`
at 46,947 Brotli bytes. Its p95 values are 30.3 ms for D100, 136.9 ms for M1K,
and 584.4 ms for M5K, with 4,735.68 retained bytes per island. These are checked
development measurements, not public release claims.

`--release --dedicated` requires B1 evidence and the exact full methodology.
The release evaluator rejects exploratory classification even when every timing
is under its cap. See [browser testing](browser-testing.md) for the distinction
between Playwright conformance and actual-product qualification.

## Validated S1 evidence

S1 is Linux x86-64 with exactly eight selected dedicated vCPUs, at least 16 GiB
RAM, the `performance` CPU governor, warm filesystem cache, and loopback
providers. The benchmark can inspect every condition except whether the vCPUs
are dedicated, so a qualifying runner must supply that explicit attestation:

```sh
rtk env \
  SUPRNOVA_LIVE_S1_CPUSET=0-7 \
  SUPRNOVA_LIVE_S1_DEDICATED=1 \
  SUPRNOVA_LIVE_REQUIRE_S1=1 \
  CARGO_INCREMENTAL=0 \
  scripts/run-snapshot-budget.sh
```

`SUPRNOVA_LIVE_REQUIRE_S1=1` makes missing S1 evidence fatal. Without it, an
otherwise valid result is labelled `local_exploratory`; the benchmark never
infers dedicated CPUs or promotes arbitrary hardware to S1. The JSON record
includes CPU model, architecture, selected affinity, memory, kernel, governor,
Rust compiler, release profile, warmup and sample counts, fixture SHA-256, and
the observed percentiles. Its machine-readable contract is
`benchmarks/s1-environment.schema.json`.

A passing `local_exploratory` result qualifies the benchmark implementation for
an internal development iteration. A validated S1 result is required before the
first release or public performance claim. The distinction keeps development
honest without making access to dedicated benchmark hardware a feature-delivery
dependency.

B1 is the separate browser environment contract recorded in
`browser/benchmarks/environments/b1.json`. Browser measurements use that name;
S1 continues to name the Rust server benchmark environment above. Neither
classification is inferred from a successful local gate.
