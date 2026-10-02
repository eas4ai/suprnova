# Shared Live library contract

Prefix: LCT

The developer agreed these six requirements through the Live SDK decision on
2026-10-01. They govern the shared structural validator and its public boundary.
The [schema detail](live-library-schema.md) is part of these requirements.
Live specification 20 owns the component-library boundary; runtime-template
changes to specifications 01 and 19 are a later commitment.

## Requirements

[LCT-001] Suprnova shall own a public `suprnova-live-library-contract` crate for non-executing package validation. It shall not depend on the HTTP framework or internal Live engine. Application consumers shall use `suprnova::live` and `suprnova::view` for framework behavior.
Falsifier: An external consumer needs an internal engine import, the crate's dependency graph includes either forbidden implementation dependency, or source inspection finds a second validator in the SDK. Compile an independent consumer and inspect Cargo metadata and imports.
Mechanism: live-library-public-boundary
Status: Agreed 2026-10-01

[LCT-002] The shared validator shall enforce schema 1's closed catalog and component-manifest structures, namespace and version rules, license expression, file roles and order, dependency graph, element declarations, and closed view-data schema. It shall reject duplicate JSON keys.
Falsifier: A valid package fails, or a single-rule-invalid package passes or reports the wrong stable diagnostic code. Execute the shared golden corpus through both the public crate and framework tooling.
Mechanism: live-library-schema
Status: Agreed 2026-10-01

[LCT-003] The shared validator shall inspect the complete local package inventory, validate exact payload bytes, and compute the canonical package digest defined in the schema 1 draft. Paths shall remain within the supplied root; links, special files, traversal, missing files, and undeclared files shall be rejected.
Falsifier: A forbidden path is consumed, changed bytes retain the old identity, inventory mismatches pass, or recomputation differs from an independently specified digest vector. Exercise temporary directories, link replacement, byte mutation, and fixed digest vectors.
Mechanism: live-library-filesystem
Status: Agreed 2026-10-01

[LCT-004] Validation shall enforce the draft's hard size, count, and nesting limits while reading and shall leave the package unchanged. Inspection shall not run package code, Cargo builds, scripts, imports, or network operations.
Falsifier: An over-limit input passes or is allocated at its declared size, source bytes change, or an execution/network sentinel fires. Test each boundary and one step beyond, with filesystem and process observations.
Mechanism: live-library-limits
Status: Agreed 2026-10-01

[LCT-005] Public validation and framework tooling shall return the same stable diagnostic codes for the shared valid and invalid corpus. Reports shall identify the validator version, relative source location when available, digest when available, and checked and skipped stages; terminal output shall escape metadata control characters.
Falsifier: Either entry point disagrees with golden outcomes, loses a fixture, emits unsafe control characters, or reports a skipped integration/rendering stage as passed. Compare complete fixture coverage and structured reports, including failure cases.
Mechanism: live-library-diagnostics
Status: Agreed 2026-10-01

[LCT-006] The shared structural check shall preserve existing legacy manifest semantics and explicitly distinguish structural package acceptance from runtime-template checking, binary compatibility, installation, and rendering qualification.
Falsifier: Existing legacy installer fixtures regress, a legacy package is silently reclassified as schema 1, or a structural-only report claims runtime support. Run legacy regressions and report-stage negative controls.
Mechanism: live-library-legacy
Status: Agreed 2026-10-01

## Proof boundary

The framework supplies independently authored golden diagnostics and digest
vectors, a public crate, and a supported tooling entry point. The external
consumer probe lives outside the framework workspace while it builds and runs.
It compares all shared cases and inspects the actual dependency graph.

Observer controls establish that the checks detect their stated mismatches.
They are not framework results. Missing implementation and incomplete collection
remain unverified. Completion requires the real collector, current passing
observations, regression checks, and the Sudus review/report gates.

The schema stage checks declared metadata and view-data schema structure only.
Runtime-template parsing, registered Rust projection compatibility, installation,
and rendering remain separately identified stages. Structural success cannot
be presented as successful integration or framework compatibility.
