# Iteration 005 implementation ledger

This ledger records implementation checkpoints for the integrated Suprnova Live
authority. It is evidence about the current implementation state, not a
replacement for the normative Iteration 005 contract.

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
