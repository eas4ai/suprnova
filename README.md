# Suprnova Live

This repository is the development workspace for the future internal
`suprnova-live` crate. Iteration 001 completed the trusted interaction spine:
bounded canonical JSON, purpose-separated signed public-seed and instanced
snapshots, verified hydration and deterministic dehydration, Tier 0 revision
authority, bounded seed promotion, strict Live v1 request/response contracts,
and shared Rust/TypeScript conformance fixtures. The active iteration 002
contract adds the standalone host-neutral server-component, Askama view, action,
validation, checker, harness, and endpoint-service kernel.

It is not a third-party crate and is not yet the application-facing Live API.
The eventual public facade belongs under `suprnova::live`. Actual Suprnova
router/HTTP/view/session/CSRF/auth/tenant adapters and final macro placement wait
for the atomic integration move; the browser DOM runtime remains iteration 003.

## Workspace

- `src/` contains the Rust 2024 internal engine contracts.
- `browser/` independently checks the Live v1 data contract in strict
  TypeScript. It is a conformance package, not the browser runtime.
- `fixtures/v1/` is the single reviewed cross-language fixture corpus.
- `docs/specs/suprnova-live/` is the normative specification set.
- `docs/implementation/` records what completed iterations actually implement.
- `fuzz/` owns one nightly target per external parser/verifier.
- `benchmarks/` records the A8/16 budget contract and measured evidence.

The workspace uses Rust 1.91.1 with no default features. Dependencies are exact
in `Cargo.lock`; browser tooling is exact in `browser/package-lock.json`.

## Verification

Run the unattended gate from the repository root:

```sh
rtk env CARGO_INCREMENTAL=0 scripts/gate.sh
```

The gate checks the specs and optional Fable archive, license inventory, Rust
format/lints/tests/doctests/MSRV, cross-language fixtures, fuzz target builds,
strict TypeScript checks, browser byte budgets, and the local A8/16 snapshot
budget. It reviews warnings without using blanket `-D warnings`.

See [snapshot v1](docs/implementation/snapshot-v1.md),
[protocol v1](docs/implementation/protocol-v1.md), the
[threat model](docs/implementation/threat-model-v1.md),
[fixture process](docs/implementation/fixtures.md), and
[benchmark reproduction](docs/implementation/benchmarking.md).
