# Live kernel benchmark reproduction

All checked results are versioned evidence, not marketing claims. The current
repository results are `local_exploratory`; validated S1 evidence remains the
qualification boundary for a release or public performance statement.

## Snapshot-processing benchmark

The iteration 001 benchmark measures the complete trusted snapshot pipeline for
the named `A8/16` workload: verify, hydrate, deterministic dehydrate,
canonicalize, and sign. Its state is exactly 8 KiB, its response-size check uses
16 KiB of HTML, and component hooks, rendering, domain I/O, and providers are
outside the timed region.

Run a local exploratory measurement from the repository root:

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

Run it from the repository root:

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
all resolve generated runtime paths through the standalone final-facade fixture.
The budget uses pinned nightly expansion for token/byte counts and isolated
MSRV `cargo check` work for each fixture:

```sh
rtk node scripts/check-expansion-budget.mjs
```

The checked local evidence records 1,762/15,622/154,222 expanded tokens,
10,174/92,884/919,984 expanded bytes, and 5,372/5,400/5,562 milliseconds of
isolated check work for 1/10/100 components. The gate rejects fixture drift,
expansion size more than 10% above the checked baseline, and token or byte
growth above twelve times between consecutive fixtures. Isolated check time is
bounded only within one run: each larger fixture must finish within twice the
1-component fixture's check time. Dependency compilation dominates that check,
so a per-component compile regression that matters shows up as that ratio, and
the same-run ratio cancels machine speed, `CARGO_BUILD_JOBS`, and concurrent
load, none of which a checked millisecond baseline can. The recorded
milliseconds and job setting are exploratory context, are never compared
against the checked baseline, and are never presented as release-grade
toolchain performance. `tests/expansion_budget_rules.mjs` holds the rule
contract.

## Browser runtime benchmark

The browser budget has two layers. `rtk npm --prefix browser run budget` is the
unattended regression check: it rebuilds the exact production artifacts, reports
both core variants' Brotli sizes, enforces response/snapshot and optional-artifact
caps, requires the checked baseline to name the exact ESM artifact, and evaluates
all applicable hard/regression limits. Core size has no absolute ceiling until a
completed implementation provides evidence for one. The check does not rerun noisy
wall-clock measurements.

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
