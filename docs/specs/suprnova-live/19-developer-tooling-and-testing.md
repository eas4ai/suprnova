# Suprnova Live -- 19 Developer Tooling and Testing

Status: Normative design specification
Last revised: 2026-08-24

## Scope

This domain owns procedural-macro diagnostics, generated metadata, the
cross-language view checker, CLI scaffolding, component and cache test harnesses,
browser/conformance/security testing, observability, benchmarking, and upgrade
diagnostics. It consumes contracts from all runtime domains and makes them
verifiable; it does not redefine those contracts or permit code generation to
hide runtime behavior.

## Capabilities

### Rust metadata generation and diagnostics

Procedural macros and derives shall generate the repetitive component, state,
action, validation, event, and schema metadata required by Live while producing
Rust-native diagnostics for invalid declarations.

Acceptance criteria:
- Generated code has a documented expansion contract and does not require
  application developers to implement unsafe dispatch or serialization glue.
- Duplicate names, unsupported types, conflicting attributes, invalid lifecycle
  signatures, and inaccessible actions fail with useful spans.
- Metadata includes stable component, field, action, event, view, and version
  identities consumed by checking and tests.
- Generated APIs remain inspectable through supported tooling and do not hide
  security-critical defaults.
- Macro UI tests cover valid, invalid, and migration cases.
- Compile-time work and generated binary size are benchmarked and bounded.

UX flow:
1. Application developer declares a valid component -> generated metadata wires
   it into Live with minimal boilerplate.
2. Declaration violates a contract -> compilation points to the source and
   explains the accepted shape.

### Cross-language view checker

Suprnova shall check external templates against generated Rust component and
runtime directive metadata during build/check workflows. The checker shall
validate contracts it can prove and clearly identify dynamic cases it cannot.

Acceptance criteria:
- Checks cover view existence, action/event names, model paths and permissions,
  modifiers, directive grammar, required keys, nested island ownership,
  feedback targets, effect names, component-library anatomy, and selected
  accessibility invariants.
- Askama is the normative checked template grammar and source model; the checker
  consumes its compatible parser/AST rather than maintaining a semantically
  unrelated approximation.
- A future view engine supplies its own checker adapter and passes the shared
  view/directive conformance suite before claiming checked Live compatibility.
- Diagnostics identify template path, line/column or source region, component,
  and violated contract where available.
- Unknown dynamic template expressions do not produce false claims of proof;
  they require explicit supported escape or runtime validation.
- The checker does not require no-JavaScript action parity.
- Editor/JSON diagnostic output is machine-readable and stable enough for IDE
  integration.
- Checker grammar/version matches the shipped runtime through conformance data.

UX flow:
1. Application developer runs the normal check command -> Rust and template
   contracts are evaluated together.
2. Template references `doesNotExist` or forbidden model state -> checking fails
   before the defect is discovered by clicking in a browser.

### CLI scaffolding and inspection

Suprnova CLI shall scaffold conventional Live components, views, tests, and
component-library usage and shall inspect registered contracts without
overwriting application code unexpectedly.

Acceptance criteria:
- Scaffolding creates Rust, external template, and test files in documented
  application locations.
- Namespaces, component names, routes, and view paths are validated before
  writing.
- Existing files require explicit conflict handling and are never silently
  overwritten.
- Generated examples use secure current APIs, semantic HTML, and accessible
  feedback.
- Inspection commands list safe component/action/model/event/cache metadata
  without secrets.
- Dry-run output makes planned changes reviewable.

UX flow:
1. Application developer requests a new Live component -> CLI previews or
   creates the conventional files and next commands.
2. Target conflicts or input is invalid -> no partial scaffold remains and the
   CLI explains recovery.

### Component test harness

Rust tests shall mount components with controlled context, propose allowed model
values, call actions, advance lifecycle, and assert rendered HTML, validation,
events, effects, redirects, snapshots, authorization, and failures without a
real browser where browser behavior is not under test.

Acceptance criteria:
- Tests can set identity, tenant, route, locale, session, time, dependencies,
  and application services explicitly.
- APIs distinguish browser proposals from direct trusted state setup.
- Assertions cover visible/absent text or DOM, field errors, dispatched events,
  effects, redirect targets, revision advancement, and no unexpected errors.
- Snapshot state inspection redacts secrets and verifies integrity/tamper
  rejection.
- Concurrency, duplicate, timeout, and stale-revision scenarios are
  deterministic.
- Tests cover public seed promotion, promotion limits, `refresh_on_promote`,
  transient models, one committed outcome per base revision, and permitted
  method reinvocation after a rolled-back database-coupled claim.
- Test failures explain the relevant lifecycle and rendered difference.

UX flow:
1. Application developer writes a focused Live test -> harness executes the
   server component loop deterministically.
2. Assertion fails -> output identifies state/action/render phase and a bounded
   semantic diff.

### Browser and conformance testing

Runtime, morphing, navigation, uploads, accessibility, and component-library
behavior shall have browser tests across a defined support matrix. Shared
fixtures shall prove Rust, protocol, checker, and JavaScript agreement.

Acceptance criteria:
- Browser tests cover keyboard, pointer/touch where material, focus/selection,
  forms, file inputs, nesting, reorder, transitions, reduced motion, offline,
  history, bfcache, push reconnect, and CSP.
- Protocol fixtures are consumed by Rust and JavaScript implementations.
- Morph fixtures include adversarial conditional/list/nested/widget cases.
- Browser fixtures prove redirect precedence, commit-after-morph, explicit
  no-render handling, and fresh-render recovery without action replay after a
  post-acceptance morph failure.
- Tests run with deterministic clocks/network schedules where possible.
- End-to-end upload/SSE/WebSocket/poll tests serve the exact production-mode
  ESM, classic-script, and typed asset-manifest outputs; TypeScript source,
  development transforms, and test-only bundles are not release evidence.
- Accessibility automation supplements, but does not replace, manual
  assistive-technology review for critical components.
- Supported browser/version policy is explicit and reproducible in CI.

UX flow:
1. Framework contributor changes a runtime contract -> conformance and browser
   suites exercise downstream observable behavior.
2. Cross-language output diverges -> CI identifies the incompatible fixture and
   owning spec.

### RenderCache test harness

Tests shall assert policy, classification, keys, variance, dependencies,
generations, Complete/Composite handling, hit/miss paths, stitching, deployment
tiers, stale behavior, singleflight, and multi-principal/multi-node safety
without relying on sleep or opaque provider state.

Acceptance criteria:
- Deterministic stores, clocks, generation authority, and rebuild coordinators
  are injectable.
- Tier 0 is the behavioral reference suite; database and networked key/value
  adapters pass the same semantics plus advertised multi-node/failure fixtures.
- Tests can assert route/ORM/template bypass on a proven hit.
- Multi-principal and multi-tenant fixtures detect private-output leakage.
- Write/rollback/generation behavior and stale windows are controllable.
- Concurrency tests prove one fenced publication while permitting only bounded
  duplicate computation under injected lease expiry or partition.
- Race fixtures force data changes across consistent read, render, fresh
  generation reread, and publication boundaries.
- Failure diagnostics show policy and dependency reasoning without body leaks.

UX flow:
1. Application developer tests a cached route -> harness exposes why it hit,
   missed, bypassed, stitched, or rebuilt.
2. Dependency or privacy expectation fails -> assertion identifies the observed
   dimension/generation that changed the decision.

### Observability and production diagnostics

Live and RenderCache shall emit correlated traces, metrics, logs, and optional
safe diagnostic events that expose work, latency, failures, queueing, morphing,
cache proof, and rebuild behavior without leaking state or unbounded labels.

Acceptance criteria:
- Correlation links document request, island, wire request, action, render,
  morph outcome, cache lookup, and rebuild where applicable.
- Metrics cover latency, payload/body size, queue depth, error category,
  connection state, cache layer/outcome, validation age, staleness, and rebuild
  fan-in using bounded dimensions.
- Logs redact snapshots, signatures, cookies, CSRF/upload tokens, action secrets,
  and private cached body content.
- Development diagnostics can be richer only through explicit trusted mode.
- Hooks integrate with Suprnova's established observability facilities.

UX flow:
1. Operator investigates a slow or failed interaction -> correlated telemetry
   shows which bounded phase consumed time or failed.
2. Diagnostic detail would expose private state -> system records safe metadata
   and preserves redaction.

### Architecture performance budgets, benchmarks, and release gates

Performance claims shall be based on realistic canonical documents, Live
interactions, cache layers, invalidation storms, and multi-node workloads rather
than hello-world routing alone. Architecture performance budget v1 in
`00-overview.md` assigns explicit workloads, environments, hard limits, and
regression thresholds to the browser runtime and critical server paths so
"small" or "fast" does not remain an untestable adjective.

Acceptance criteria:
- Fixtures include realistic DB/ORM/template/auth/feature work and meaningful
  HTML sizes.
- Benchmarks cover cold render, warm L1, warm L0, conditional 304, island action,
  morph cost, uploads, invalidation storm, singleflight, and multi-node
  coherence.
- Browser budgets cover compressed runtime bytes, bootstrap time, idle CPU and
  observers, incremental memory per connected island, protocol/snapshot
  overhead, and morph latency across documented DOM node/depth classes.
- Server budgets cover action-path database/provider round trips, snapshot
  encode/verify cost, allocations and byte copies, Complete-hit and
  Composite-assembly latency, dependency final-reread cost, and tail latency.
- `crates/suprnova-live/benches/render_cache_budget.rs` owns the Complete L0
  allocation assertion. It runs the hot-hit workload in an isolated serialized
  benchmark process, resets a benchmark-only counting global allocator after
  warmup, and fails when the measured request exceeds four heap allocations.
  Shared-byte identity and an instrumented body adapter separately prove that
  response construction performs no full-body copy.
- Memory allocation/copies, tail latency, throughput, body size, and database
  work are measured where relevant.
- Baselines and regression thresholds are versioned and reproducible.
- Every budget names its reference workload, environment, percentile, allowable
  variance, and release-blocking threshold rather than one context-free number.
- Security and correctness assertions run alongside performance benchmarks so
  unsafe shortcuts cannot qualify.
- Release notes identify compatibility, migration, and benchmark changes.

UX flow:
1. Framework contributor changes architecture -> benchmark suite compares the
   affected realistic paths to baselines.
2. Regression exceeds policy -> release gate reports the workload and metric
   instead of hiding it behind aggregate throughput.

## Iteration 001 and 002 harness placement

The shared v1 corpus lives at repository-root `fixtures/v1/` and is documented
in [`fixtures.md`](../../implementation/fixtures.md). Rust consumes every case
through `tests/golden_fixtures.rs`; the strict TypeScript conformance package
consumes the same repository-relative files through
`browser/tests/golden-fixtures.test.ts`. Both enumerate all case kinds and verify
the exact ordered `manifest.sha256`; neither keeps a second expected-value
table. The TypeScript package is conformance infrastructure, not the iteration
003 DOM runtime.

Iteration 002 adds `fixtures/v2/` under the same manifest-driven corpus
contract for lifecycle operations and response fields that v1 cannot represent.
Rust and TypeScript enumerate both version directories from one harness and keep
no parallel expected-value implementation; server-only component behavior stays
in Rust integration fixtures rather than pretending TypeScript executes Rust.

External-boundary hardening lives in `tests/parser_properties.rs`,
`tests/fuzz_regressions.rs`, and `tests/security_boundaries.rs`, with nightly
targets under `fuzz/fuzz_targets/` for canonical input, signed snapshots,
update requests, and update responses. Tier 0 ledger and promotion concurrency
tests use barriers and injected clocks rather than sleep.

The A8/16 executable is `benches/snapshot_budget.rs`; its schema, checked
result, reproduction script, and explanation live under `benchmarks/`,
`scripts/run-snapshot-budget.sh`, and
[`benchmarking.md`](../../implementation/benchmarking.md). It times verify,
hydrate, deterministic dehydrate, canonicalize, and sign for exact 8 KiB state,
with 500 warmups and 40 post-warmup batches. It fails above 500 microseconds p95
or the fixed 1 KiB control and 768-byte snapshot overhead caps. Result metadata
distinguishes validated S1 from local exploratory hardware and requires an
explicit dedicated-vCPU attestation before claiming S1.

`scripts/gate.sh` is the unattended iteration gate. Its shell contract rejects
blanket `-D warnings` and omission of Rust/TypeScript fixture parity,
security/fuzz coverage, or either budget. The gate runs the spec/archive and
license checks, Rust format/Clippy/tests/doctests/MSRV, nightly fuzz build,
strict TypeScript install/format/lint/type/test/build/budget, and a scratch-file
A8/16 measurement without rewriting the checked baseline.

## Iteration 002 harness placement

Iteration 002 adds an internal procedural-macro development crate, generated
component/view/state/action metadata, macro UI fixtures, an Askama-compatible
view checker, and a host-neutral component harness. Final macro placement in
`suprnova-macros`, public `suprnova::live`/`suprnova::view` paths, CLI wiring,
and generated-application checks wait for the atomic integration move. No
generated application code may name the standalone development crate as a
product dependency.

The checker validates view existence, Askama source structure, component and
action identities, model paths and permissions, lifecycle signatures, binding
and URL metadata, stable child keys, nested ownership, and the server-visible
portion of Live directive grammar. Dynamic cases produce an explicit unproved
result or supported escape; they never receive a false success. The component
harness controls trusted host context, services, sessions, transactions, time,
randomness, ledger state, mount parameters, model proposals, action calls,
authorization, rendering, and semantic outcomes without a browser.

Shared fixtures extend v1 with metadata, lifecycle, binding, action, validation,
view-checker, endpoint-service, and hostile-host-adapter cases. Deterministic
concurrency tests cover ledger/transaction commit and rollback, duplicate
requests, parameter propagation, and registry races. A named host-neutral
`A8/16` action-framework benchmark records complete local environment metadata
and enforces the 2-millisecond p95 architecture cap; validated S1 evidence
remains first-release qualification rather than an internal iteration blocker.

The checker has two explicit syntax layers. Exact `askama_parser` 0.16 parses
Askama nodes, expressions, includes, inheritance, control-flow branches, and
source spans. Exact `html5ever` 0.39 tokenizes the resulting bounded static HTML
regions and directive attributes. The checker walks Askama branches separately,
joins only compatible HTML/island stack states, preserves source locations, and
marks dynamic tag/attribute structure unproved. It does not maintain a second
home-grown Askama grammar or pretend a flat rendered sample proves every branch.

Macro expansion always names the final `::suprnova::live` facade and hidden
`::suprnova::live::__private` contract. Standalone macro UI tests compile against
a dedicated dev-only facade fixture with those exact paths. Production
expansion never names `suprnova_live`, the development macro crate, or a
test-only path, preventing successful standalone tests from concealing final
integration drift.

## Iteration 004 harness placement

Iteration 004 extends the host-neutral Rust harness with revisioned temporary
uploads, a quarantined reverse-proxy/file provider, a provider-neutral
direct-storage conformance adapter, signed subscription descriptors, and
controlled SSE, WebSocket, and polling transports. Clocks, randomness, ports,
network order, chunk/message delivery, provider failures, scanning, replay, and
shutdown are injectable. Host adapters are conformance apparatus and do not
claim active Suprnova storage or broadcasting registration.

The shared manifest-driven corpus advances to `fixtures/v4/` for promoted
`live:upload`, `live:progress`, `live:poll`, and `live:stream` grammar,
capability negotiation, independently versioned upload/event envelopes,
transition cases, compatibility, and redacted diagnostics. Rust, the Askama
checker, and TypeScript enumerate the same manifest. Existing Live request and
response protocol v1/v2 fixtures remain unchanged rather than being copied or
renumbered as a generic protocol v3.

Browser end-to-end tests serve
`browser/dist/suprnova-live.esm.js`,
`browser/dist/suprnova-live.classic.js`,
`browser/dist/suprnova-live.uploads.esm.js`,
`browser/dist/suprnova-live.uploads.classic.js`,
`browser/dist/suprnova-live.async.esm.js`,
`browser/dist/suprnova-live.async.classic.js`, and
`browser/dist/suprnova-live.assets.json` exactly as produced by the deterministic
release build. The reference host exercises actual chunked HTTP, the direct
provider contract, authorized SSE/WebSocket, fallback polling, and verified
Live refresh. Source modules, dev-server transforms, and test-only bundles may
support unit diagnosis but cannot satisfy browser release evidence.

Upload and asynchronous-update adversarial suites cover quota exhaustion,
forged handles, transfer-grant sentinel leaks, chunk/finalization races,
scanning timeout, cleanup, revoked subscriptions, sequence gaps, replay
overflow, reconnect storms, slow consumers, fanout pressure, late delivery,
page lifecycle, and bfcache. New codecs and transition entry points receive
property and fuzz coverage. Deterministic barriers and injected clocks replace
sleep-based correctness.

`U4/16`, `E100/1K`, and `R100` record the architecture budget's exact optional
artifact, retained-memory, buffered-byte, scheduler, progress/event dispatch,
queue, and reconnect limits on `S1`/`B1`. The build gate enforces 45 KiB Brotli
for each core variant, 20 KiB for each upload variant, and 16 KiB for each async
variant. Runtime workloads enforce the formula/count/latency caps in the
overview and the existing 15-percent regression policy. A larger application
upload limit may increase stored file bytes but may not authorize unbounded
framework memory, queues, connections, or diagnostic retention.

## Acceptance criteria

- Rust, templates, protocol, and browser runtime share checkable generated
  contracts.
- CLI scaffolding is safe, conventional, and non-destructive.
- Server and browser harnesses verify functional, security, accessibility, and
  concurrency behavior.
- RenderCache correctness and privacy are directly assertable.
- Observability and realistic benchmarks expose performance without secrets or
  hello-world theater.
- Explicit runtime, morph, protocol, cache, and server budgets provide
  reproducible release gates.

## Decisions and revisions

- 2026-08-24 -- Added production ESM/classic Stimulus adapter artifacts to the
  exact-browser evidence set. Tests prove core excludes Suprnova bridge and
  lifecycle modules, all production artifacts exclude `@hotwired/stimulus`, and
  real Stimulus conformance uses built core plus built adapter rather than
  TypeScript source.
- 2026-08-23 -- Iteration 004 extends the shared corpus to version 4 and the
  host-neutral harness to real chunked HTTP, provider-neutral direct storage,
  authorized SSE/WebSocket, and polling fallback. Browser completion evidence
  serves the exact deterministic core plus optional upload/async ESM/classic
  artifacts selected through the typed manifest. Adversarial, fuzz/property,
  lifecycle, `U4/16`, `E100/1K`, and `R100` evidence remains in the unattended
  gate without relaxing existing architecture caps.
- 2026-08-21 -- Locked the checker to Askama 0.16 AST plus html5ever 0.39 HTML
  tokenization with branch-state joins and explicit unproved dynamics. Locked
  macro output to final `::suprnova::live` paths tested through a dev-only facade
  fixture rather than development-crate paths.
- 2026-08-21 -- Assigned macro metadata, Askama-aware checking, the host-neutral
  component harness, expanded conformance corpus, and action-framework budget
  to iteration 002. Final macro/facade/CLI placement and real Suprnova adapter
  integration wait for the atomic move.
- 2026-08-21 -- Fixed iteration 001 harness placement: one shared v1 fixture
  corpus, four parser/verifier fuzz targets, the A8/16 release benchmark and S1
  schema, and one unattended cross-language gate with a checked shell contract.
- 2026-08-21 -- Assigned the Complete L0 allocation and copy budgets to the
  isolated `render_cache_budget.rs` harness with a benchmark-only counting
  allocator and shared-byte identity instrumentation.
- 2026-08-21 -- Stage 5 established architecture performance budget v1 in the
  overview with explicit environments, canonical workloads, absolute caps, and
  repeatable regression blocking.
- 2026-08-21 -- The view checker validates Live contracts but does not require
  synthesized no-JavaScript action paths.
- 2026-08-21 -- Benchmarks measure avoided work in realistic applications, not
  merely raw hello-world routing speed.
- 2026-08-21 -- Askama is the normative checked template substrate; alternate
  engines require checker adapters and shared conformance.
- 2026-08-21 -- Tier 0 is the provider behavioral reference; higher tiers add
  fault/topology suites without changing semantics.
- 2026-08-21 -- Required Stage 5 to set explicit versioned browser-runtime,
  morph, protocol, action, and RenderCache performance budgets and release
  thresholds; architecture performance budget v1 fulfills that requirement.
