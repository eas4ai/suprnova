# Browser runtime verification

## Test layers

The browser workspace is installed only with `npm ci` from the exact lockfile.
Generation check, Prettier, ESLint, TypeScript, and Vitest run before building.
Vitest covers bounded parsers, generated directive parity, shared v1/v2/v3
fixtures, schedulers, models/forms, transport, response ordering, signals,
feedback, extensions, navigation, morph preflight/controls, lifecycle,
properties, security boundaries, and deterministic build contracts.

After the production build and byte-identical `build:check`, Playwright runs the
same checked artifact on pinned Chromium, Firefox, and WebKit projects. The
suite covers bootstrap/duplicate loading, seed/lazy behavior, nested and
multiple islands, delegated events, local signals, optional Stimulus, effects,
models/forms/IME, focus/selection, hostile DOM, response order, morph identity,
feedback/accessibility, CSP, network faults, recovery, navigation/transitions,
bfcache, resource lifecycle, leaks, and browser workload instrumentation.

Rust separately consumes every shared fixture, checks the Askama/directive
contract, runs regression/property/security suites and macro UI tests, and
builds every nightly fuzz target. Directive and browser-metadata fuzz targets
exercise the cross-language browser boundary; crashes become named regression
seeds rather than unreviewed fixture truth.

## Actual browser qualification

Playwright is not actual-browser floor evidence. Its Chromium and WebKit
engines are implementation test targets, not attestations for Chrome, Edge, or
Safari product releases. Release support requires fresh provider-neutral,
authenticated case receipts for Chrome 111/current, Edge 111/current, Firefox
128/current, and Safari 16.4/current, bound to the exact runtime and fixture
hashes.

`npm run compatibility:check -- --allow-unqualified` is the honest local gate:
missing or stale external evidence reports `unqualified` without pretending a
test failure or release pass. A release gate omits that flag and fails unless
all eight targets qualify. Malformed, mismatched, simulated, user-agent-only,
or failing evidence is always a failure.

## Budgets and diagnostics

`npm run budget` verifies snapshot/control overhead, the 45 KiB Brotli core
cap, exact current artifact identity, and the checked browser baseline.
`npm run budget:browser` records D100 connection plus M1K/M5K morph workloads
using five warmups, thirty samples, thirty seconds idle, and 4x CPU throttling
by default. The harness also records retained bytes per island with the counting
allocation/heap instrumentation described by its schema.

Local measurements are `exploratory`; `--release --dedicated` additionally
requires the pinned B1 environment and methodology. An exploratory pass is
useful regression evidence but is never called B1. See
[benchmark reproduction](benchmarking.md) for current values and commands.

Playwright traces are retained on failure. The lifecycle test port exposes
closed observer/listener/timer/transport/controller counts, and heap/DevTools
inspection is corroboration rather than the normative leak oracle. Diagnostics
must remain closed and redacted in every mode. The complete unattended command
is `rtk env CARGO_INCREMENTAL=0 scripts/gate.sh`; set
`SUPRNOVA_LIVE_RELEASE=1` only in a release-qualified environment.
