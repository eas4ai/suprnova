# Shared Live contract proof

This directory will hold the real collector for the
[six agreed requirements](../docs/spec/live-library-contract.md).
`collect.py` is not implemented. `sdk.json` deliberately has no checkout selected.
Normal observations therefore remain unverified. No Rust validation, SDK
integration, installation, runtime-template, or rendering result is claimed.

## Collector invocation

The observer will execute:

```sh
python3 -B proof/collect.py LCT-001 --nonce RUN_NONCE \
  --framework-identity sha256:FRAMEWORK_BYTES --sdk-identity sha256:SDK_BYTES
```

The collector must perform the observations during that invocation. It writes
one JSON object to stdout and returns zero only when collection is complete.
Its top-level fields are `schema: 1`, `kind: "framework"`, `requirement`, `nonce`,
`framework_identity`, `sdk_identity`, and `observation`. The identities and nonce
must match the arguments. A collection error is nonzero, not a fabricated result.
Logs belong on stderr; both streams count toward the 1 MiB runner limit.

Set `sdk.json` to `{"schema":1,"checkout":"PATH"}` under an action lease.
The path may be absolute or relative to this repository. Identity is a hash of
current source bytes and executable flags, including dirty and non-ignored new
files. The framework capture covers the paths listed by `FRAMEWORK_INPUTS` in
`live_contract_checks/run.py`, including an existing ignored Cargo lockfile and
local Cargo config. The SDK capture covers its Git inventory. Both are checked
again after collection. This freshness identity is not a package digest.

The collector must also capture every extra consumed build input, including
resolved dependencies, generated sources, external Cargo configuration, feature
selection, environment, and build commands. Do not claim reproducibility from a
Git revision or these source hashes alone. Build into temporary output directories
and preserve command results without modifying the captured source inputs.

## Required observations

The executable field checks are in `live_contract_checks/acceptance.py`.
`control_examples.py` illustrates shapes only; it is never a data source for a
real observation.

| Requirement | Real work and observation |
| --- | --- |
| LCT-001 | Build an independent consumer outside this workspace using the public contract crate, `suprnova::live`, and `suprnova::view`. Record its exit code, direct dependencies, contract dependency closure, and internal imports. Review current SDK source for duplicated validation rules. Record reviewed relative files, source citations, and the captured SDK identity in `ownership_review`. |
| LCT-002 | Discover the complete golden corpus and run each fixture through the public crate and supported framework tooling. Record `corpus` and one `cases` entry per fixture with `id`, independent `expected` code, `public`, and `tooling` outcomes. Use `ok` for accepted cases. |
| LCT-003 | Run the filesystem corpus with temporary real directories, payload changes, traversal, links, link replacement, special files, and inventory mismatches. Record the matrix and at least two independently specified `digest_vectors` with different expected package identities. |
| LCT-004 | Run at-limit and over-limit cases for all eight limits. Record package identities `before` and `after`, measured process/network/file-write counters, and instrumented bounded reads for manifest, payload, and total package bytes. Each read probe records `id`, `limit`, `read`, oversized `declared_size`, and `largest_allocation`. |
| LCT-005 | Run the entire schema corpus through both entry points. Record the matrix and one report per case with `id`, `validator_version`, `digest`, relative `location`, `checked`, `skipped`, and captured `terminal` output. Use a relative fixture source when parsing fails before a finer location exists. |
| LCT-006 | Run the legacy corpus and discover and execute the existing installer regression tests. Record the matrix, discovered/passed/failed test names, `legacy_kind: "legacy-manifest"`, `catalog_kind: "schema1-library"`, and report stages. |

Every structural report must list `structure` as checked and runtime, binary,
installation, and rendering as skipped. Valid cases require a package digest;
invalid cases may report null when identity is unavailable. The observer allows
64 KiB of bounded read-ahead for byte-limit probes and rejects allocation of the
oversized declared length. This is an observation allowance, not a larger package
limit. Both boundary outcomes still use the exact contract limits.

Process and network counters must observe the validator call, excluding the
collector's own test and build setup. Use containment or instrumentation that
would detect an attempted package side effect. Zero counters without observation
are not evidence. The collector is trusted reviewable test code; JSON assertions
alone cannot prove how its values were obtained.

## Completion evidence

Before claiming a requirement passes, inspect the collector, independent golden
expectations, real consumer source, dependency results, and complete case coverage.
Run the affected Rust tests and project checks. Missing prerequisites remain
unverified. The commitment also requires Sudus mechanism review, requirement
receipts, implementation review, and the independent report before Done.
