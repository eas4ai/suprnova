# Suprnova Live -- Conventions

Status: Normative
Last revised: 2026-08-23

## Authority and application

These conventions govern Suprnova Live design, implementation, generated code,
browser assets, component-library artifacts, tests, references, and
documentation. The repository-level `AGENTS.md`, the machine production coding
standard it imports, and any future repository-local `BEST_PRACTICES.md` remain
authoritative. A repository-local `BEST_PRACTICES.md` overrides the machine
copy.

This repository is the dedicated development workspace for the future internal
Suprnova Live crate, not a specification-only repository or a third-party crate.
Iteration 001 creates implementation here beside the normative
`docs/specs/suprnova-live/` directory and `scripts/check-specs.mjs`; it does not
modify the active Suprnova checkout. Development remains here until the separate
workspace materially blocks integration, testing, or a coherent change. At that
trigger, the product tree, normative specifications, and checker move together
into `suprnova/crates/suprnova-live/` in one controlled integration, and this
repository ceases to be a maintained authority. The large `reference/` catalog
and optional `suprnova-live.zip` Fable handoff export are non-normative
development artifacts and need not move. When the ZIP exists, the checker
requires its Markdown bytes to match the source set exactly. No Stage 5 edit
authorizes modification of the active Suprnova workspace; a later integration
iteration must do that explicitly.

## Implementation standards

### Completeness and scope

- The active implementation contract is [`iterations/004.md`](iterations/004.md).
  Closed contracts remain historical evidence; preliminary sequencing in an
  older contract does not override the active confirmed boundary.
- Implement the active iteration contract completely. Do not substitute an
  MVP, placeholder, TODO, empty adapter, unverified scaffold, or narrower
  behavior for an agreed capability.
- A change owns its whole contract: implementation, public facade, macro/checker
  metadata, browser behavior, tests, documentation, generated templates,
  reference impact, and migration note where applicable.
- Attractive adjacent work is not implicit scope. Record it through
  `/next-iteration` after Stage 6 rather than coupling it to the current change.
- Existing unrelated work in Suprnova, Magnetar, or this workspace is preserved.
  Never rewrite, revert, reformat, or commit another contributor's changes merely
  to simplify Live work.

### Rust safety and API design

- Public application-facing APIs live under `suprnova::live` or
  `suprnova::view`; consumers never import `suprnova-live` directly.
- The internal engine crate does not depend on the `suprnova` facade. Framework
  services enter through narrow adapter traits and typed contexts to prevent a
  crate cycle and make conformance fixtures independent.
- Public operations return typed `Result` values and preserve causal errors.
  Panics are limited to proven internal invariants in tests or unreachable
  generated states; hostile input, provider failure, and application mistakes
  are never panic paths.
- Production code contains no `unsafe`. A later proposal requiring `unsafe`
  needs a separately approved, documented safety case and cannot enter as an
  implementation convenience.
- Public items have useful rustdoc because `suprnova` denies missing docs and
  broken or private intra-doc links.
- Clippy warnings are reviewed and resolved, but commands and gates do not use
  blanket `-D warnings`. An intentional lint suppression uses the narrowest
  practical scope and `#[allow(clippy::lint_name, reason = "why this is safe")]`.
  Crate-wide or category-wide allowances require a dated specification decision.
- Prefer owned immutable values and explicit state transitions. Interior
  mutability, global state, and dynamic typing require a demonstrated boundary
  need and tests for lifecycle and concurrency.

### Errors and diagnostics

- `LiveError` and subordinate enums distinguish protocol, validation,
  authentication, authorization, CSRF, snapshot, revision, render, morph,
  provider, cache, upload, compatibility, and internal failures.
- Error conversion preserves a stable machine category, safe recovery
  instruction, source context, and causal chain. Do not flatten errors to strings
  or use status codes as the only taxonomy.
- Production messages contain no snapshot bytes, signatures, cookies, tokens,
  transient models, private HTML, SQL values, stack traces, or policy internals.
- Developer diagnostics point to the Rust declaration, template path and source
  region, directive, component, island, lifecycle phase, and correlation ID when
  available.

### Async execution and concurrency

- Tokio tasks are structured and cancellation-aware. Detached tasks require an
  explicit owner, shutdown path, bounded queue, and observability.
- Never hold a blocking mutex, database transaction, provider lease, or mutable
  component borrow across an unrelated `.await`.
- Every queue, batch, upload, body, parser, recursion depth, retry, lease,
  connection, and diagnostic buffer has a configured bound.
- Timeouts and cancellation do not imply rollback of an external effect.
  Transaction, idempotency, outbox, and delivery semantics remain explicit.
- The instance-ledger contract guarantees at most one committed accepted
  outcome per base revision. Action bodies are safe to invoke again before
  commit and must not claim exactly-once external behavior.

### Rendering and templates

- Routes and components render through `suprnova::view`. Askama is the normative
  checked grammar, but Askama-specific types do not leak into handler or
  component signatures unless the facade explicitly owns the wrapper.
- Templates are external `.html` files. They contain presentation conditions,
  loops, includes, layouts, semantic markup, and declarative attributes, not
  authorization decisions, database access, or arbitrary JavaScript.
- Escaping is default. Trusted HTML uses one explicit audited type and cannot be
  constructed from untrusted text through a convenience conversion.
- A render is deterministic for declared inputs and dependency generations.
  Locale, time, randomness, feature state, configuration, assets, and identity
  that affect bytes enter the render context and dependency/variance machinery.
- A failed render publishes no partial document, island, snapshot, cache entry,
  header set, or success outcome.

### State, protocol, and cryptography

- Component fields are private to the server unless generated metadata marks
  them model-bindable. Locked, server-only, computed, and transient categories
  are distinct types or metadata states, not naming conventions.
- Protocol structs use `serde` with explicit field names, deny duplicate or
  unknown required fields as specified, and reject unbounded collections before
  expensive allocation.
- Public JSON field names use `snake_case`. Rust uses the same semantic names;
  TypeScript adapters may expose idiomatic local names only behind generated
  codecs and conformance fixtures.
- Signed snapshot bodies use the versioned canonical JSON profile. Do not sign
  `serde_json` output whose order or numeric representation is incidental.
- Snapshot keys are derived per purpose and version with HKDF-SHA-256 and sign
  with HMAC-SHA-256. Verification uses explicit key IDs, bounded rotation
  windows, and constant-time comparison.
- Protocol, snapshot, directive, view metadata, and cache-entry fixtures are
  consumed by both Rust and TypeScript tests. A handwritten duplicate schema is
  not a second source of truth.

### RenderCache and providers

- RenderCache is separate from the generic application cache. It stores typed
  Complete or Composite representations and proof metadata, never arbitrary
  handler values masquerading as a response.
- `RenderStore`, `LiveInstanceLedger`, `RebuildCoordinator`, and
  `GenerationLedger` remain independent traits. A provider implements only the
  capabilities it can prove.
- Tier 0 is the behavioral reference. Tier 1 and Tier 2 reuse the same semantic
  suite and add topology-specific CAS, lease, fencing, eviction, partition, and
  failure fixtures.
- Generation truth is database-authoritative at every tier. Hints, memory, Redis,
  Memcached, files, and blob stores may accelerate observation but do not become
  correctness authority.
- Cache keys and metrics contain stable purpose-specific digests, never raw
  cookies, session IDs, principal secrets, arbitrary URLs, or high-cardinality
  values.
- Complete hot hits retain shared immutable bytes through response construction.
  A full-body clone, handler call, template render, or database query on that path
  is a performance defect unless the owning spec explicitly requires it.

### Browser runtime

- Runtime source is strict TypeScript targeting ES2020 and is built into
  deterministic versioned ESM and classic-script artifacts with source maps kept
  out of production responses by default.
- The universal core and optional upload/async ESM/classic feature pairs are
  selected only through trusted rendered roles and the typed asset manifest.
  Optional loading deduplicates and registers through the core lifecycle rather
  than starting another runtime or accepting element-selected artifact URLs.
- Application developers can use the shipped runtime without Node, npm, a
  bundler, Stimulus, or a client component framework. Bundler integration is an
  optional delivery choice for the same artifact and protocol.
- The core runtime owns `live:` parsing, local signals, scheduling, transport,
  response ordering, registered effects, and the Suprnova morph adapter.
  Stimulus is loaded only when an application chooses custom controllers.
- Event handling is delegated where semantics permit. Island, controller,
  observer, listener, timer, and upload resources connect and dispose exactly
  once.
- No `eval`, `new Function`, server-returned script, inline expression language,
  or monkey-patch of private Idiomorph/Stimulus state is permitted.
- DOM writes use semantic platform APIs, preserve trusted types/CSP contracts,
  and pass oldest-supported plus current-browser fixtures.

### Components and accessibility

- Official components begin with semantic native HTML. Custom behavior is added
  only where the native element cannot satisfy the specified interaction.
- Component presentation uses Tailwind CSS 4 utilities and versioned semantic
  theme tokens. Raw palette values and component-private design constants do not
  become public theme APIs.
- Each component documents anatomy, variants, sizes, states, keys, Live/local
  ownership, keyboard behavior, accessible name/description, focus behavior,
  reduced motion, and morph continuity.
- WCAG 2.2 AA is the baseline. Automated accessibility checks do not replace
  manual keyboard and assistive-technology review for critical components.

### Observability and performance

- Use Suprnova tracing and metrics facilities. Spans and metrics correlate work
  through bounded identifiers and never record payload bodies or secrets.
- Fast paths have named workloads, explicit provider work, allocation/copy
  expectations, p50/p95 data, and checked-in baselines. Hello-world throughput
  alone cannot substantiate a performance claim.
- Benchmark changes run correctness and security assertions beside performance
  measurements. A faster path that weakens coherence, privacy, authorization,
  revision, or recovery semantics fails.
- Architecture performance budget v1 in `00-overview.md` is release-blocking.
  Budget revision and implementation optimization are separate changes unless
  the developer explicitly approves them together.

### Testing strategy

- Unit tests own pure codecs, keys, state machines, parsers, classification, and
  provider primitives. Property and fuzz tests own external parsers and
  canonical round trips.
- Macro UI tests own valid/invalid declarations and source diagnostics.
  Golden fixtures are small, reviewed, and updated only with an explained
  contract change.
- Integration tests own middleware ordering, sessions, CSRF, authorization,
  transactions, ORM generations, real providers, rendering, CLI scaffolds, and
  dogfood application flows.
- Browser tests own DOM identity, forms, selection, IME, focus, controllers,
  signals, transitions, uploads, offline/retry, history, bfcache, CSP,
  accessibility, and old/new runtime compatibility.
- Concurrency tests use deterministic barriers, injected clocks, and controlled
  providers rather than sleep-based probability.
- Every defect receives a failing regression test at the lowest layer that can
  prove it, plus a higher-level test when the failure crossed a subsystem
  boundary.

## Naming and organization

### Development and eventual integration layout

```text
suprnova-live/
  Cargo.toml
  docs/specs/suprnova-live/
  scripts/
    check-specs.mjs
    gate.sh
  src/
    component/
    state/
    snapshot/
    protocol/
    render/
    render_cache/
    providers/
    testing/
  browser/
    src/
    tests/
    package.json
    package-lock.json
  components/
    templates/
    styles/
    catalog/
  fixtures/
  benches/
    render_cache_budget.rs
  reference/                 # development evidence; not integrated

suprnova/
  crates/suprnova-live/      # eventual destination of the product tree,
                             # normative specs, and checker together
  framework/src/live/
  suprnova-macros/src/live/
  suprnova-cli/src/commands/live/
  suprnova-cli/src/templates/files/live/
  app/
  manual/
```

The internal crate may split a module only when it has a coherent owned
contract; directory count is not a goal. Shared helpers live with their owning
domain unless two independent domains require the same stable abstraction.

The top level of `docs/specs/suprnova-live/` is closed to the 26 numbered
specifications plus `conventions.md`, `glossary.md`, and `ux.md`. Supplemental
normative material, such as a threat model, lives in a named subdirectory and is
linked from the numbered specification that owns its requirements; adding it
also requires extending the checker and any present handoff archive contract
deliberately.

Iteration contracts live in `iterations/NNN.md`. The checker validates their
numeric name, project/iteration title, scope-contract status, agreed ISO date,
required sections, links, text hygiene, and exact bytes in any present handoff
archive. Capture for a future decision lives under `iterations/next/` and does
not become the current contract until promoted through `/next-iteration`.

### Rust and generated names

- Crates and modules use `snake_case`; types and traits use `UpperCamelCase`;
  functions, fields, actions, and events use `snake_case`; constants use
  `SCREAMING_SNAKE_CASE`.
- Public framework types prefer the `Live` or `Render` qualifier only when the
  unqualified term would collide with an existing Suprnova concept.
- Procedural attributes use concise lower-case names such as `#[live]`,
  `#[action]`, `#[model]`, `#[locked]`, and `#[server_only]`. Their generated
  contract identities are fully qualified and versioned.
- Standalone Live macro expansion names only final `::suprnova::live` and
  `::suprnova::live::__private` paths. A dev-only facade fixture supplies those
  exact paths to macro UI tests; production expansion never names the
  development engine or macro packages.
- Error variants name the violated contract rather than the current
  implementation, for example `SnapshotExpired` rather than `HmacFailed` when
  expiry is the public outcome.
- Test names describe observable behavior and expected outcome. Avoid issue
  numbers or implementation function names as the only explanation.

### Templates, directives, and components

- Application Live templates use `.html` and reside in the conventional
  application view tree selected by `suprnova::view`; generated examples use a
  `live/` subdirectory without making that path part of component identity.
- `live:` directive names and modifiers use lower-case kebab form. Public action,
  field, event, and effect values map to generated stable names; arbitrary Rust
  paths never appear in HTML.
- DOM keys are stable logical identities, not list indices, random render values,
  timestamps, database display text, or mutable labels.
- Official component names describe semantic roles. Variant names describe
  purpose or emphasis rather than hard-coded color, pixel value, or current
  visual appearance.
- Theme tokens use Tailwind CSS 4 namespaces where they intentionally generate
  utilities and `--suprnova-*` semantic CSS variables for component roles that
  must remain stable across palettes.

### Protocol, cache, and storage names

- Media types, endpoint metadata, protocol fields, snapshot forms, cache entry
  kinds, generation keys, and provider capabilities use versioned constants from
  one Rust source of truth and generated TypeScript fixtures.
- Provider keys begin with a versioned purpose namespace and hash unbounded or
  sensitive dimensions. Key construction has golden tests and never depends on
  debug formatting.
- Database objects use the `suprnova_live_` or `suprnova_render_` prefix and
  reversible timestamped migrations. Migration names state the durable contract,
  not the backing product.
- Telemetry names begin with `suprnova.live.` or `suprnova.render_cache.` and use
  bounded enumerated attributes.

## Dependency and version policy

- Cargo and npm lockfiles are ground truth for exact transitive versions.
  Overview versions identify intentional architecture lines, not an alternative
  dependency inventory.
- Askama, Idiomorph, Stimulus, Tailwind CSS, canonicalization, and cryptographic
  changes require upstream changelog/license/security review plus Live
  conformance, bundle-size, and migration evidence.
- Idiomorph is vendored or locked into the shipped runtime artifact. Application
  package resolution cannot silently replace it with an incompatible version.
- The oldest supported browser matrix moves only in a dated normative revision.
  Optional APIs remain feature-detected even when all current browsers implement
  them.
- Rust MSRV follows the Suprnova workspace. Runtime TypeScript and npm versions
  are pinned in the internal browser package and updated deliberately.

## Verification commands

Commands below are run from the named repository root. A check is reported as
passing only when that exact command ran successfully. Heavy Cargo commands are
never run concurrently with another build in the Suprnova tree.

### Specification workspace: `/home/shawn/workspace2/suprnova-live`

Per documentation change:

```bash
node scripts/check-specs.mjs
git diff --check
```

While the optional Fable handoff ZIP is present in the development workspace,
regenerate it before the structural check so its Markdown bytes remain exact:

```bash
(cd docs/specs && zip -X -q -FS -r suprnova-live.zip suprnova-live -i '*.md' -x 'suprnova-live/iterations/next/*')
node scripts/check-specs.mjs
```

Before a Stage commit:

```bash
node scripts/check-specs.mjs
git diff --check
git status --short
```

The Same Page Stop hook runs
`.agents/skills/new-project/scripts/spec-drift-gate.mjs`; it supplements rather
than replaces the explicit structural check.

### Dedicated Live workspace: `/home/shawn/workspace2/suprnova-live`

While iterating on Rust:

```bash
CARGO_INCREMENTAL=0 cargo check --all-targets --all-features
CARGO_INCREMENTAL=0 cargo test <test-filter>
```

After a coherent Live task:

```bash
CARGO_INCREMENTAL=0 cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features
CARGO_INCREMENTAL=0 cargo test --all-targets --all-features --no-fail-fast
```

Before any push from the dedicated workspace:

```bash
CARGO_INCREMENTAL=0 scripts/gate.sh
```

### Suprnova integration workspace: `/home/shawn/workspace2/suprnova`

These commands apply only after the migration trigger moves Live into the
Suprnova workspace. While iterating on an integrated change:

```bash
CARGO_INCREMENTAL=0 cargo check -p suprnova-live
CARGO_INCREMENTAL=0 cargo check -p suprnova
CARGO_INCREMENTAL=0 cargo test -p suprnova-live <test-filter>
CARGO_INCREMENTAL=0 cargo test -p suprnova --test <affected-live-file>
```

After a coherent integrated task:

```bash
CARGO_INCREMENTAL=0 cargo fmt --all --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets
CARGO_INCREMENTAL=0 cargo test -p suprnova-live --no-fail-fast
CARGO_INCREMENTAL=0 cargo test -p suprnova --test <affected-live-file>
CARGO_INCREMENTAL=0 cargo test -p suprnova-macros
CARGO_INCREMENTAL=0 cargo test -p suprnova-cli --test template_drift
```

After a public API or generated-template change:

```bash
CARGO_INCREMENTAL=0 cargo test -p suprnova-cli --test scaffold_snapshot -- --ignored
```

Before any push from the integrated workspace:

```bash
CARGO_INCREMENTAL=0 scripts/gate.sh
```

Before a release or when provider/security/MSRV/feature behavior changes:

```bash
CARGO_INCREMENTAL=0 scripts/gate.sh --full
```

### Browser runtime: `browser/`

The path becomes `crates/suprnova-live/browser/` only after integration.

Dependency installation after checkout or lockfile change:

```bash
npm ci
```

Per runtime task:

```bash
npm run format:check
npm run lint
npm run typecheck
npm test
npm run test:browser
npm run build
npm run budget
```

`build` must reproduce checked artifacts byte-for-byte from the lockfile and
source. `budget` measures the production artifacts and architecture performance
fixtures; it is not a source-file-size approximation.

### Provider and browser matrix checks

Provider conformance, oldest-browser, current-browser, accessibility, CSP, and
benchmark matrix commands shall be wired into `scripts/gate.sh` or a script it
invokes before the corresponding implementation can be called complete. Tests
requiring Redis, Memcached, PostgreSQL, MySQL/MariaDB, or a real browser remain
explicit and unattended; credentials are never embedded in commands or
fixtures.

## Decisions and revisions

- 2026-08-23 -- Advanced the active contract to iteration 004 as one complete
  standalone upload and asynchronous-update foundation across specs 08 and 14.
  Kept upload and event protocols distinct over shared bounded-resource
  lifecycle machinery; split upload/async into manifest-selected optional
  ESM/classic artifacts to preserve the 45 KiB universal core cap; required
  provider, continuity, adversarial, and hard resource-budget evidence; and
  retained storage/broadcast framework adapters for the later atomic Suprnova
  integration.
- 2026-08-22 -- Advanced the active contract to iteration 003 as one complete
  standalone browser interaction runtime across specs 09 through 13. Retained
  vertical implementation milestones inside the single contract; rejected both
  splitting a coherent runtime into artificial numbered partial products and
  calling a bootstrap-only shell complete. `agent-browser` and DevTools MCP may
  assist exploratory diagnosis, while committed Playwright, shared-fixture, and
  benchmark evidence remain the completion authority.
- 2026-08-21 -- Locked standalone macro expansion to final
  `::suprnova::live` paths and required a dev-only facade fixture, preventing
  successful development builds from concealing public integration drift.
- 2026-08-21 -- Advanced the active contract to iteration 002 and kept its
  server-component kernel standalone. Conformance host adapters are test
  apparatus, not actual Suprnova integration; the latter waits for the atomic
  code/spec/checker move.
- 2026-08-21 -- Added checked nested iteration contracts to the structural and
  optional handoff-archive gates; `iterations/NNN.md` is normative scope while
  `iterations/next/` remains unconfirmed capture.
- 2026-08-21 -- Applied the house warning policy: Clippy findings are reviewed
  without blanket `-D warnings`, and intentional suppressions are scoped and
  reasoned rather than hidden by broad allowances.
- 2026-08-21 -- Reserved the top-level spec directory for the checked canonical
  set; supplemental normative documents live in linked subdirectories and must
  be added to the checker/archive contract deliberately.
- 2026-08-21 -- Kept iteration 001 development, normative specifications, and
  the checker colocated in this dedicated workspace. Migration into Suprnova is
  triggered only by a material integration/testing/coherence blocker and then
  moves code, specs, and checker together; reference sources and the optional
  Fable handoff ZIP remain non-normative development artifacts.
- 2026-08-21 -- Established one internal engine crate with public framework,
  macro, CLI, dogfood, and browser/component integration points; rejected
  application dependencies on internal crates.
- 2026-08-21 -- Chose strict TypeScript and reproducible npm scripts for runtime
  contribution while shipping prebuilt artifacts so application adoption needs
  no JavaScript toolchain.
- 2026-08-21 -- Made Tier 0 the provider semantic reference and the architecture
  budgets release-blocking rather than advisory.
- 2026-08-21 -- Mirrored Suprnova's no-unsafe, documented-public-API, typed-error,
  targeted-test, full-gate, and non-concurrent-build rules.
