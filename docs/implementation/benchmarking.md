# Snapshot-processing benchmark

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
