# Iteration 005 implementation ledger

This ledger records implementation checkpoints for the integrated Suprnova Live
authority. It is evidence about the current implementation state, not a
replacement for the normative Iteration 005 contract.

## 2026-09-02 -- Live CLI workflows and the application tooling protocol

The Suprnova CLI gained `live:make`, `live:check`, `live:inspect`, and
`live:assets`, closing plan Task 9. `live:make` scaffolds a component in
`src/live/`, its view in `templates/live/`, and its registration in a
`registry()` builder in `src/live/mod.rs`, declares `pub mod live;` in
`src/lib.rs`, validates every target and refuses traversal and symlinks
before writing, writes atomically, never overwrites, rolls back every file a
failed run had written, and reports a dry run.
The other three commands are thin clients of a new hidden framework console
command, `__suprnova:live-tool`, registered at link time by
`framework/src/live/tooling.rs`; the CLI starts it through the explicit
console-binary Cargo wrapper and consumes the bounded, versioned JSON-lines
protocol in `framework/src/live/tooling_protocol.rs`. The helper owns
registry access, checked-template validation through the engine
`TemplateChecker`, safe inspection (presence booleans and counts only), and
asset export with lengths and digests; the CLI keeps no framework or engine
dependency and re-verifies every digest, version, sequence, identity, cap,
and marker on the transport, failing closed with no writes on anything
unsupported, stale, truncated, oversized, or unexpected on stdout, and never
echoing stdout content. `live:assets` stages `<out>/<identity>/` and renames it
into place, treats an identical publication as up to date, and refuses drift
unless `--replace` is given. The engine registry gained `ComponentRegistry::names`
so the framework can enumerate registered components.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_tooling_protocol
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_cli --test live_scaffold --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --lib live_
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test console --test console_typed --test console_db_seed --test command_macro --test live_boot --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p app --test console_binary_e2e --test console_greet
rtk cargo fmt --all -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova-cli -p suprnova -p suprnova-live --all-targets --all-features
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
```

Those commands passed eight framework tooling-protocol cases (a registered
hidden helper, proved and failing components, bounded redacted inspection,
byte-exact asset export, unsupported protocols and operations, template root
and symlink refusals, and missing views instead of a vacuous pass), 26 CLI
cases across the three new suites plus six unit cases (help, project and
template-root preconditions, hostile stream matrix, caps, a fake application
console replaying scripted streams for check, inspect, and assets, scaffold
conflicts, dry run, idempotence, invalid names, symlink refusal, rollback of
a failed run, exact idempotent publication, drift refusal and replacement,
and digest mismatches), the existing console and Live suites, zero new Clippy findings,
and the documentation contracts. The generated application's bootstrap does
not yet bind the registry; plan Task 10 wires that so a fresh scaffold passes
`live:check` out of the box.

## 2026-09-02 -- Framework artifact delivery and document bootstrap

Suprnova now serves the exact reviewed browser artifacts and emits typed
bootstrap markup from documents, closing plan Task 8. The ten deterministic
build outputs are tracked under `browser/dist/` and embedded into the engine
by the new `suprnova_live::artifacts` module, which validates the manifest
against the embedded bytes on first use and fails closed on any drift in
digest, length, file name, role, capability, or version. The Live gate gained a
"tracked artifact parity" phase that rejects a rebuilt `dist/` differing from
the tracked bytes, so the embedded bytes and the reproducible build cannot
diverge silently.

`Router::try_live()` registers `/__live/v1/assets/<asset_identity>/<file>` for
`GET` and `HEAD` with immutable caching, strong digest validators, conditional
requests, `nosniff`, closed misses, and two framework-owned external boot
scripts, so a document loads no inline executable code and a strict
`script-src 'self'` policy holds. `LiveDocument::bootstrap` maps mounted
components to the upload and asynchronous roles, adds the Stimulus bridge on
request, emits the inert configuration element plus ordered preload and script
tags with integrity values for the ESM or classic strategy, and rejects a
second bootstrap or a mount after bootstrap. `Router::try_live_document`
declares a document route without startup mounts. Two engine additions
support the host: `TrustedHtml::framework_generated` for framework-assembled
markup and the public `TrustedLiveRequestContext::host_scope_facts` accessor
from Task 7. The reference host's artifact validation was not replaced; the
engine module is the shared home a later cleanup can point it at.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --test runtime_artifacts --test trusted_markup
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_public_api --test live_facade_contract --test live_dependency_topology --test live_document_routes --test live_routes --test live_boot --test live_hostile_adapter --test live_view_contract
rtk cargo fmt -p suprnova -p suprnova-live -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live --all-targets --all-features
(cd crates/suprnova-live/browser && rtk npm run format:check && rtk npm run lint && rtk npm run typecheck)
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/build-contract.test.ts tests/optional-artifacts.test.ts)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/framework-bootstrap.spec.ts --project=chromium --project=firefox --project=webkit)
(cd crates/suprnova-live && rtk git diff --exit-code --stat -- browser/dist)
rtk tests/gate_contract.sh
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
```

Those commands passed six engine artifact and trusted-markup cases, seven
framework asset and bootstrap cases plus four unit cases, 49 cases across the
eight existing Live suites, zero new Clippy findings, 18 browser artifact unit
cases, and the nine-case real-server Playwright scenario on Chromium,
Firefox, and WebKit (an example binary, `live_bootstrap_host`, is the real
Suprnova server the Playwright configuration starts on port 4177). The
scenario covers ESM and classic role selection, a core-only document, the
optional Stimulus role, duplicate boot tags, an incompatible optional
feature, an integrity failure that leaves SSR content intact, a strict
self-only Content Security Policy, and byte-exact immutable artifacts with
conditional requests. The full Live crate gate and the Suprnova repository
gate were not rerun for this checkpoint; the repository gate runs before the
next push.

## 2026-09-02 -- Framework asynchronous transport routes

Suprnova now registers the reserved versioned `/__live/v1/async/subscriptions`,
`/__live/v1/async/memberships`, `/__live/v1/async/events`, and
`/__live/v1/async/socket` paths next to the action and upload endpoints, using
the existing router, middleware chain, response, and WebSocket upgrade
machinery. Components declare `streams(...)` in the `#[live]` attribute and the
macro emits the engine's subscription metadata. The framework installs the
engine's subscription registry, authorization, continuity, and credential ports
only for asynchronous requests; the engine signs every descriptor, verifies
every membership, and drives bounded document delivery. Stream authorization is
the Gate ability `live:{component}.stream.{stream}`, application code publishes
through `suprnova::live::LiveStreams`, and the route, credential, limit, and
failure contracts are recorded in `docs/implementation/async-updates.md`.

Two engine accessors became public for the host: `SubscriptionError::new` and
`TrustedLiveRequestContext::host_scope_facts`. The production browser artifact
and the async reference host now use the versioned SSE and WebSocket paths. The
framework's WebSocket upgrade path records its pre-chain `Origin` proof through
a new `record_passed_before_chain` attestation entry, because that check runs
before the middleware chain and therefore cannot claim a position in the
enforced execution order. Engine document sessions sit behind per-transport
asynchronous locks so engine callbacks into the host ports never re-enter the
runtime's table mutex.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_async_backpressure --test live_async_routes --test live_async_security
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_async_backpressure --test live_async_routes --test live_async_security --test live_boot --test live_dependency_topology --test live_document_routes --test live_external_authoring --test live_facade_contract --test live_hostile_adapter --test live_macro_expansion --test live_public_api --test live_routes --test live_trusted_context --test live_upload_policy --test live_upload_providers --test live_upload_routes --test live_upload_security --test live_view_contract
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::async_transport
rtk cargo fmt -p suprnova -p suprnova-live -p suprnova-live-test-support -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live -p suprnova-live-test-support --all-targets --all-features
(cd crates/suprnova-live/browser && rtk npm run format:check && rtk npm run lint && rtk npm run typecheck)
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/async-connections.test.ts tests/async-feature.test.ts)
(cd crates/suprnova-live/browser && rtk npm run build && rtk npm run build:check)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/async-lifecycle.spec.ts --project=chromium)
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
rtk git diff --check
```

The three new framework suites passed 14 cases on two consecutive runs, the
complete framework Live sweep passed 102 cases across 18 binaries, the four
transport parser unit cases passed, and 69 browser async unit cases, the
deterministic artifact check, and the five Chromium async lifecycle cases
passed against the rebuilt artifact and the reference host. Clippy reported
zero errors and no new warnings; the previously reviewed
`execution/service.rs` argument-count warning and the pre-existing test-module
notes outside the Live tree remain. The fairness assertion in
`live_async_backpressure` is a liveness bound (the sibling is served within the
backlog admitted before it joined) because kernel socket buffering makes a
tighter interleaving bound nondeterministic through a real socket; the
coalescing assertion is checked only after envelopes were read, which is the
state barrier proving the document drained. The full integrated gate was not
rerun for this checkpoint.

Before the branch was first pushed to `origin` on 2026-09-02, the Suprnova
repository gate (`scripts/gate.sh`, default tier: formatting, published
document references, dash policy, workspace Clippy, JSON rustdoc,
`cargo test --workspace --no-fail-fast`, Magnetar all-feature tests, Postgres
regressions, and scaffold compile tests) passed on commit `7c2a1123`. Reaching
that took four follow-up commits: the gate's reference checker no longer reads
ignored-name fragments as grep options; the crate's per-directory assistant
guidance file and the two `docs/superpowers` plans left the published tree
(they remain local, ignored files) and `conventions.md` states the authority
rule inline; the spec checker spells its em-dash test as an escape; and every
cookie-queue test that drives the session middleware now holds the crypt hook
guard, which removed a one-in-six parallel failure that predated this branch.
The Live crate's own gate under `crates/suprnova-live/scripts/gate.sh` was not
run in this session.

## 2026-09-02 -- Standalone synchronization and budget removal

The integrated crate merged the final standalone `main`, commit `59395ec`,
through a subtree merge on top of the `6d19d02` import. The merge brought the
WebSocket closure classification fix, the reference-host policy-close
handshake, the same-run bound for the macro expansion check, the removal of
every benchmark and artifact budget from `scripts/gate.sh`, and the deletion
of the artifact budget script together with its reviewed size history. The
provenance-graph hardening this crate had layered on that script left with
it. `npm run build` now prints the raw and Brotli bytes of every artifact and
nothing caps them; the budget scripts remain on-demand tools. Captured
future-iteration notes stayed out of the import as the contract requires.

Dedicated S1 and B1 qualification is release-checklist work outside Iteration
005, and the historical-baseline question is closed because the size history
it concerned no longer exists. The "Qualification still outstanding" paragraph
in the cutover checkpoint below is superseded. The documentation contract had
required the singular `## Child parameter envelope` heading, the removed
standalone disclaimer in the component-authoring document, and the earlier
Stimulus exclusion wording; the contract now names the plural heading, the
real `suprnova::live` facade statement, and the reworded Stimulus sentence.

Verification completed from the integration worktree:

```bash
(cd crates/suprnova-live && bash tests/gate_contract.sh)
(cd crates/suprnova-live && bash tests/documentation_contract.sh)
(cd crates/suprnova-live && node scripts/check-implementation-docs.mjs)
(cd crates/suprnova-live && node scripts/check-specs.mjs)
(cd crates/suprnova-live && node tests/expansion_budget_rules.mjs)
rtk cargo test -p suprnova-live --test upload_budget_contract --test async_budget_contract
rtk cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
(cd crates/suprnova-live/browser && npm run format:check && npm run typecheck)
(cd crates/suprnova-live/browser && rtk proxy npm run lint)
(cd crates/suprnova-live/browser && npm run test:unit -- tests/budget-contract.test.ts tests/build-contract.test.ts tests/package-contract.test.ts tests/protocol-overhead.test.ts tests/async-websocket-closure.test.ts)
(cd crates/suprnova-live/browser && npm run build)
(cd crates/suprnova-live/browser && npx playwright test e2e/bootstrap.spec.ts --project=chromium)
git diff --check
```

Those commands passed the gate contract, four Rust budget-contract cases, all
28 reference-host cases, twenty focused browser unit cases, the deterministic
build, and the twelve Chromium bootstrap cases. `rtk npm run lint` from this
nested package resolves the system ESLint instead of the package's pinned one
and fails before linting; the unfiltered `rtk proxy npm run lint` passed. The
full integrated gate did not run for this checkpoint.

## 2026-09-01 -- Framework upload boundaries and application reacquisition

Suprnova now registers the versioned `/__live/v1/upload` control/data endpoint
and exposes an explicit router helper for authenticated application-owned
reacquisition paths outside `/__live/`. Generated Live component metadata owns
checked per-field upload policy, including count, declared and aggregate bytes,
accepted media, and replacement behavior. The public `suprnova::live` facade
exposes application configuration and typed policy/host contracts without
leaking the internal engine crate.

Host-owned adapters keep revisioned lifecycle persistence, bounded metadata,
quarantine byte I/O, reverse-proxy transfer, constrained direct-provider
instructions and reports, scanner and application validation, immutable
evidence, finalization, and cleanup separate. The engine remains authoritative
for handle identity, transfer grants, state transitions, ready proposals, and
finalization semantics. Every request revalidates current mount and principal,
session, tenant, component, field, and document scope; a per-handle operation
lock serializes chunk, completion, cancellation, action, finalization, and
cleanup races. Chunk bodies reserve the shared in-flight budget before
buffering, carry an explicit authoritative offset, reject impossible permit
requests, and preserve exact idempotent outcomes without writing bytes before
revision acceptance.

Action dispatch retains only signed ready-handle proposals, commits the Live
outcome before durable finalization, and reconciles retryable finalizer failure
without invoking the action again. Cleanup runs automatically and owns bounded
retry/lease behavior. The browser and Rust host now agree on the versioned
route, `queued` create state, chunk-response shape, and required offset header.
The Rust reference host's ordinary-action fixture was also corrected to emit a
typed base64url correlation identity and the normative v2 `invoke_action`
operation; that correction turned seven shared-host regressions into a green
26-case suite.

Verification completed from the integration worktree:

```bash
rtk cargo test -p suprnova --test live_upload_routes --test live_upload_security --test live_upload_providers
rtk cargo test -p suprnova-live --test upload_file_provider --test upload_service --test upload_direct_provider --test upload_protocol --test upload_state --test upload_validation --test upload_budget_contract --test upload_identity --test upload_finalization --test upload_cleanup --test upload_security --test upload_framework_budget_integrity
rtk cargo test -p suprnova --test live_upload_policy
rtk cargo test -p suprnova-macros --test live_ui
rtk cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/upload-*.test.ts)
(cd crates/suprnova-live/browser && rtk npm run build)
(cd crates/suprnova-live/browser && rtk npm run build:check)
(cd crates/suprnova-live/browser && rtk npm run budget)
(cd crates/suprnova-live/browser && rtk npm run budget:upload)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/uploads.spec.ts --project=chromium)
rtk cargo clippy -p suprnova -p suprnova-live -p suprnova-macros --all-targets --all-features
```

Those commands passed 22 framework route/security/provider cases, 98 engine
upload cases, two policy cases, the macro UI suite, all 26 reference-host cases,
134 browser upload unit cases, deterministic artifact checks, the existing
artifact and upload budget gates, and the Chromium upload lifecycle. Clippy
reported zero errors and retained the two previously reviewed
`execution/service.rs` argument-count warnings; no blanket warning denial or
new suppression was introduced.

## 2026-09-01 -- Exact-child delivery through the real endpoint

Accepted protocol-v2 parent execution now derives changed-child transitions
from rendered composition, binds each v2 envelope to the accepted successor
lineage, and prepares one deterministic delivery for each changed surviving
child. Unchanged, removed, replaced/remounted, duplicate/invalid-lineage, and
v1-parent outcomes emit none. Envelope signing, response encoding and bounds,
and complete response sealing all occur before host commit and ledger
acceptance, so precommit failure exposes zero response bytes and cannot accept
the parent.

The browser validates the complete parent response, morphs and commits the
successor, then pairs its one top-level signed parent snapshot with each child
delivery. The resulting ordinary scheduler intent sends the child's own current
snapshot and exact `child_parameters` carrier
`{"envelope":...,"parent_snapshot":...}` without raw parameters or a second
queue. Redirect, malformed response, morph failure, stale/mismatched boundary,
unchanged hash, and removal schedule nothing.

The existing Suprnova Live action route now parses exact/bounded carriers and
independently verifies child snapshot, parent snapshot, and purpose-separated
v2 envelope before kernel dispatch. The kernel consults authoritative parent
ledger currentness: logical missing/stale/mismatched authority is concealed,
while provider failure retains an unavailable failure. Modern
`params_changed` consumes only `EligibleChildParametersV2`, hydrates and invokes
the generated lifecycle once, renders/signs a successor child snapshot,
advances the child's ledger, and records the applied parent revision in owner
lineage. The macro generates both modern and explicitly historical v1 hooks
from one declaration; raw v1 never enters production admission.

Focused real-route coverage proves success, exact sealed response projection,
raw-envelope/v1-shaped and malformed rejection, forged signature, cross-child,
cross-session, cross-tenant, and superseded-parent rejection before component
work or child-ledger acceptance. Rejected child delivery leaves the already
accepted parent revision unchanged.

## 2026-09-01 -- Accepted-revision, signed lineage, and exact-child foundation

The host-neutral engine now exposes a provider-neutral
`LiveInstanceLedger::current_accepted_revision` authorization read. The memory
provider performs it under the same mutex as claim and commit: Ready returns the
current revision, Pending returns its accepted base rather than its unaccepted
successor, and missing, pruned/expired, or terminal Consumed authority returns
`None`. Clock or provider synchronization failure remains `LedgerError`.
Diagnostic inspection and browser snapshots are not correctness fallbacks.

Snapshot schema v1 remains stable and recognizes the optional canonical signed
`x_suprnova_live_composition_v1` extension. It carries optional owner lineage
and bounded immediate-child entries binding parent instance/revision, stable
key, child component contract, exact child instance, and depth. Exact-shape,
identity, duplicate, mixed-authority, 256-child, depth-64, and 64-KiB bounds are
enforced before trusted use. Public seeds reject it; unknown well-formed
namespaced extensions retain the existing v1 compatibility rule.

Child-parameter schema v2 has a separate signing purpose and adds exact child
instance binding without changing v1 decoding. Server authorization returns an
`EligibleChildParametersV2` only when verified v2 data matches the signed parent
snapshot lineage and the ledger still reports the exact issuing parent
revision. Superseded revisions, foreign scope/parent/key/component/child,
missing authority, and provider errors fail closed. This foundation checkpoint
deliberately deferred framework HTTP child delivery, parent response emission,
browser scheduling, and `params_changed` execution to the slice recorded above.

Strict TDD evidence includes compile-time REDs for the new ledger read,
composition extension, and v2 envelope APIs; a behavioral RED showing a replay
was still accepted after a later parent revision; and focused GREEN suites for
ledger transitions, signed composition tamper/bounds/compatibility, exact-child
bindings, lineage eligibility, supersession, missing authority, and causal
provider failure.

## 2026-08-31 -- Atomic workspace cutover

The committed standalone history, engine, browser runtime, specifications,
checker, fixtures, tests, benchmarks, and implementation guides are now owned
only by `crates/suprnova-live/` in the Suprnova workspace. The integration branch
was reconciled with Suprnova `main` through commit `a2248c64`; concurrent
framework and Magnetar changes were merged without editing or reverting them.
Outside the imported Live tree, the public tracked worktree changes only the
root workspace manifest, root lockfile, and the checked-in cutover plan. The
separate ignored local tooling repository at
`/home/shawn/workspace2/suprnova/scripts` owns the adapter in local-only commit
`ba03b7f` (`build: gate integrated suprnova live`). That repository has no
remote, was not added to the public worktree, and was not pushed.

The following command passed from the integrated Suprnova worktree after the
authority cutover and workspace reconciliation:

```bash
rtk /home/shawn/workspace2/suprnova/scripts/check-suprnova-live.sh
```

That ordinary gate passed the specification and implementation-documentation
checkers, generated license inventory, Rust formatting and Clippy review, macro
UI suite, MSRV check, fuzz build, all-target and documentation tests, reference
host, correctness-delay scanner, TypeScript formatting/lint/typecheck, 854
browser unit tests, the Iteration 004 three-engine matrix, CSP and BFCache
coverage, the broad three-engine matrix, deterministic artifact checks, reduced
local performance workloads, expansion budget, and final diff check. The gate
reported `Suprnova Live iteration gate passed`.

The first post-merge full run encountered one non-reproducible Firefox lifecycle
observation: the reconnect assertion saw zero active connections after the
membership control had advanced. No code, timeout, retry policy, or assertion
was changed. The exact failed case then passed once and passed 20 repeated runs
from `crates/suprnova-live/browser/`:

```bash
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/async-lifecycle.spec.ts --project=firefox \
    --grep "real async transport exposes bounded semantic feedback without stealing focus")
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/async-lifecycle.spec.ts --project=firefox \
    --grep "real async transport exposes bounded semantic feedback without stealing focus" \
    --repeat-each=20)
```

The complete ordinary gate was rerun unchanged and passed, including the failed
case in the broad Firefox matrix. The isolated result is retained here rather
than hidden or converted into a weakened test. If it recurs, diagnosis must add
test-host-only lifecycle provenance before changing production behavior.

Two earlier WebKit upload-timing observations in the cutover session were also
non-reproducible and are retained here. In
`classic async missing leaves the other optional feature operational`, the
upload remained `transferring` with zero of 31 bytes observed instead of
reaching `ready` within five seconds. In
`freeze and resume preserve active upload retry authority while shutdown
cancels it`, the retried upload likewise remained `transferring` with zero of 20
bytes observed. No file or timing contract was changed. Each exact case passed
on its isolated rerun from `crates/suprnova-live/browser/`:

```bash
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/iteration-004-integration.spec.ts \
    --project=webkit \
    --grep "classic async missing leaves the other optional feature operational")
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/iteration-004-lifecycle.spec.ts \
    --project=webkit \
    --grep "freeze and resume preserve active upload retry authority while shutdown cancels it")
```

Both returned `PASS (1) FAIL (0)`, and later complete broad matrices passed the
same WebKit cases. They therefore remain disclosed nondeterministic signals, not
resolved root causes. A repeat must be investigated with test-host-only upload
progress provenance before any production or assertion change.

Change-impact and drift review ran the following commands against the reconciled
`main` comparison basis:

```bash
rtk tilth diff main..HEAD --blast --budget 12000
rtk git diff --check main..HEAD
rtk git status --short --branch
rtk git diff --name-status main..HEAD -- ':!crates/suprnova-live/**'
```

Tilth reported 889 added files, zero modified files, and 11,611 added symbols;
the imported subtree therefore dominates its deliberately large report. The
range diff check passed, and the clean pre-ledger status was
`## iteration-005-live-integration`. The path-scoped diff listed only
`Cargo.toml`, `Cargo.lock`, and the cutover plan outside the Live subtree, so no
Magnetar file or unrelated framework refactor belongs to the cutover.

GitNexus `detect_changes` ran with project
`home-shawn-workspace2-suprnova-live-integration`, base branch `main`, depth 1,
and scope `crates/suprnova-live`. It reported 889 changed files and zero impacted
pre-existing symbols because the history-preserving subtree is entirely new to
the comparison basis. This is broad import evidence, not a low-risk claim about
the already independently reviewed Live implementation.

### Qualification still outstanding

The ordinary gate truthfully reports compatibility qualification as
`unqualified (0/8)`. Iteration 004's `U4/16`, `E100/1K`, and `R100` workloads
still require qualified S1 and B1 evidence. Its historical-baseline repository-
integrity issue also still requires an explicit developer-approved normative
resolution. Local exploratory or reduced measurements do not satisfy those
release gates, and the workspace move does not relabel them as passing.

The repository cutover is complete. The separate Suprnova framework-facade,
host-adapter, and RenderCache implementation plans remain active work inside
Iteration 005.
