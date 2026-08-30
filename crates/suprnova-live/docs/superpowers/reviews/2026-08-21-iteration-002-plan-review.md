# Iteration 002 Implementation Plan Self-Review

**Reviewed:** 2026-08-21
**Plan:** [`2026-08-21-iteration-002-server-component-kernel.md`](../plans/2026-08-21-iteration-002-server-component-kernel.md)
**Verdict:** Ready to execute inline. All 25 completion conditions have an
implementation owner and fresh verification evidence requirement.

## Goal-backward review

The plan was checked from the Iteration 002 definition of done backward rather
than from the proposed modules forward.

| Review dimension | Result |
| --- | --- |
| Complete scope coverage | PASS - the final matrix maps every condition to primary tasks and concrete evidence. |
| Dependency order | PASS after remediation - metadata precedes macros; state and trust precede views/mounts; lifecycle now precedes the mount service that consumes it; composition precedes signed child authority; actions precede transaction coordination; protocol precedes endpoint assembly. |
| TDD granularity | PASS - every implementation task begins with a named failing test/fixture and ends with targeted verification before its commit. |
| Trust boundaries | PASS - browser bytes cannot construct verified snapshots, child authority, mount-catalog matches, trusted request context, registered actions, or current authorization. |
| Replay/concurrency | PASS - semantic idempotency, one accepted outcome per base revision, metadata-only duplicate recovery, and the durable-host-commit split all have deterministic tests. |
| Rendering authority | PASS - document and island result types are structurally separate; endpoint-only response metadata cannot be injected by components. |
| Final integration compatibility | PASS - macro output targets final facade paths through a fixture while actual Suprnova adapters/re-exports remain deferred. |
| Browser-runtime boundary | PASS - server lifecycle operations and response intent are implemented, but scheduling, morphing, Stimulus, local signals, navigation execution, and DOM behavior remain Iteration 003. |
| Performance/tooling | PASS - A8/16, expansion scaling, checker bounds, shared v1/v2 fixtures, fuzz targets, MSRV, licenses, docs, archive equality, and the unattended gate are owned. |
| Active-repository safety | PASS - no task edits or depends on the active Suprnova or Magnetar checkout; final inspection is read-only and no push is authorized. |

## Defects found and repaired during self-review

1. The first task order attempted to build the mount service before defining the
   component lifecycle object boundary. Lifecycle reconstruction and its tests
   now precede and share the atomic-mount task; nested composition follows.
2. The first private-mount sequence attempted to sign an instanced snapshot
   before generating its instance identity. It now generates a candidate before
   identity-bound parent/child rendering and signing, retries only identity
   conflicts, and rerenders without domain effects.
3. The first session-state wording prohibited all explicit presentation of
   session-derived data. It now prohibits dehydration and diagnostic leakage
   while allowing authorized, escaped, schema-checked component output.
4. The first idempotency wording treated object-key presentation order as
   semantic. The digest now preserves ordered operations but canonicalizes model
   proposal objects, so whitespace/key order cannot break a retry.
5. Browser and archive examples initially used subshell-first commands. They now
   begin with `rtk` and retain the machine's command discipline.
6. Separating default `State` from reusable `Public` state left public-seed
   promotion without a source for omitted model/locked/session fields. The plan
   now requires exposure-aware seed validation, a fresh repeatable/effect-free
   mount initializer, verified public overlay, then typed proposals. It also
   removes the old partial instanced snapshot from promotion output; signing the
   first instanced snapshot waits for complete state and action processing.

## Execution risks that remain deliberately test obligations

- The exact generated descriptor hook surface may need internal refactoring as
  long as macro UI fixtures, canonical metadata, and final facade paths remain
  stable.
- Askama branch-state checking is the most algorithmically delicate task. Its
  parser/token/depth/branch bounds and `Unproved` result must land before any
  broad positive fixture can be trusted.
- The host transaction and ledger cannot be one atomic resource at Tier 0. The
  fault matrix, not optimistic comments, is the authority for every split-failure
  recovery result.
- The local machine may provide only exploratory performance evidence. The gate
  must enforce the local contract honestly and continue to fail closed on any S1
  claim without the required environment attestation.

No additional feature, integration adapter, or browser behavior is required to
make the plan complete.
