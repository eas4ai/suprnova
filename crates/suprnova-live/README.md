# Suprnova Live

This repository is the development workspace for the future internal
`suprnova-live` crate. Iteration 001 established the trusted interaction spine:
bounded canonical JSON, purpose-separated signed public-seed and instanced
snapshots, verified hydration and deterministic dehydration, Tier 0 revision
authority, bounded seed promotion, strict Live v1 request/response contracts,
and shared Rust/TypeScript conformance fixtures. Iteration 002 implements the
standalone host-neutral server-component kernel: generated metadata, an explicit
registry, checked Askama views, lifecycle and state, typed actions and
validation, protocol v2, trusted host context, endpoint response intent, and a
browserless component harness. Iteration 003 implements the standalone browser
runtime: deterministic assets, closed directives, local signals, bounded
scheduling and feedback, secure transport/application ordering, DOM morphing
and interaction continuity, native navigation enhancements, lifecycle control,
actual-browser qualification contracts, and browser performance budgets.
Iteration 004 adds the standalone upload and asynchronous-update foundation:
bounded temporary uploads, quarantine and validation, provider-neutral transfer,
explicit finalization and cleanup, typed events, signed subscriptions,
multiplexed SSE/WebSocket delivery, polling, continuity recovery, backpressure,
optional production artifacts, and qualified workload contracts.

It is not a third-party crate and is not yet the application-facing Live API.
The eventual public facade belongs under `suprnova::live`. Actual Suprnova
router/HTTP/view/session/CSRF/auth/tenant adapters, final macro placement, and
asset registration wait for the atomic integration move.

## Workspace

- `src/` contains the Rust 2024 internal engine contracts.
- `browser/` contains the strict TypeScript browser runtime, deterministic
  builder, unit/property suites, three-engine Playwright host, compatibility
  qualification, and browser budget harness.
- `fixtures/v1/`, `fixtures/v2/`, `fixtures/v3/`, and `fixtures/v4/` are the reviewed
  cross-language fixture corpora.
- `docs/specs/suprnova-live/` is the normative specification set.
- `docs/implementation/` records what completed iterations actually implement.
- `fuzz/` owns one nightly target per external parser/verifier.
- `benchmarks/` records the A8/16 budget contract and measured evidence.

The workspace uses Rust 1.91.1 with no default features. Dependencies are exact
in `Cargo.lock`; browser tooling is exact in `browser/package-lock.json`.

## Verification

Run the unattended gate from the repository root:

```sh
rtk env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 scripts/gate.sh
```

The gate checks the implementation-document contract, specs and optional Fable
archive, license inventory, Rust format/lints/tests/doctests/MSRV,
cross-language fixtures, fuzz target builds, strict TypeScript checks,
deterministic browser builds, Chromium/Firefox/WebKit Playwright suites,
release-aware actual-browser evidence, browser byte/performance budgets,
snapshot and action budgets, and macro expansion scaling. It reviews warnings
without blanket warning denial.

## Implementation contracts

- [Component authoring and metadata](docs/implementation/component-authoring.md)
- [Views and the Askama checker](docs/implementation/views-and-checker.md)
- [Lifecycle, state, binding, and composition](docs/implementation/lifecycle-and-state.md)
- [Actions, validation, transactions, and outcomes](docs/implementation/actions-and-validation.md)
- [Host adapter and endpoint contract](docs/implementation/host-adapter-contract.md)
- [Protocol v2](docs/implementation/protocol-v2.md)
- [Browserless component harness](docs/implementation/component-harness.md)
- [Browser runtime](docs/implementation/browser-runtime.md) and
  [asset delivery](docs/implementation/browser-assets.md)
- [Live directives](docs/implementation/live-directives.md),
  [local reactivity](docs/implementation/local-reactivity.md), and
  [scheduling and feedback](docs/implementation/scheduling-and-feedback.md)
- [Morphing and continuity](docs/implementation/morphing-and-continuity.md),
  [document navigation](docs/implementation/document-navigation.md), and
  [browser testing](docs/implementation/browser-testing.md)
- [Uploads](docs/implementation/uploads.md),
  [asynchronous updates](docs/implementation/async-updates.md), and
  [Iteration 004 operations](docs/implementation/iteration-004-operations.md)
- [Snapshot v1](docs/implementation/snapshot-v1.md) and
  [protocol v1](docs/implementation/protocol-v1.md)
- [Threat model](docs/implementation/threat-model-v1.md),
  [fixture process](docs/implementation/fixtures.md), and
  [benchmark reproduction](docs/implementation/benchmarking.md)
