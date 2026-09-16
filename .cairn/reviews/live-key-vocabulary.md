# Review - live-key-vocabulary

commitment: live-key-vocabulary
commit: 0d0d3ce8
examined:
  - LIVE-024 against the built tree: the one reader `stableKeyOf` in `crates/suprnova-live/browser/src/directives/key.ts`, and every place the runtime resolved a stable key (morph identity, controls, preservation, teleport, upload morph, stimulus, signals, transitions).
  - The vitest fixture `tests/live-key-identity.test.ts` and the checker regression `live_key_alone_names_a_keyed_scope_and_a_duplicate_is_refused`, the mechanism's two halves; the whole browser unit suite (92 files, 889 tests), typecheck, lint and the reproducible build with the reviewed integrity pins recomputed.
  - Every library component, its vendored copy under `app/templates/suprnova-ui/`, and the dogfood galleries with the doubled key attribute removed: `live:check` proves all eleven dogfood components, and the dogfood browser suite passes on chromium (16 cases).
  - The Live specs 03, 09 and 12 revisions, the manual sentence in seven locales, the changelog entries in seven locales and the translation lock.
findings:
  - resolved: The first cut put the shared key reader in `morph/keys.ts`; the build's bundle rule refuses any optional bundle (stimulus, uploads) that imports from `src/morph/`, because morphing is core runtime. The reader moved to the leaf module `directives/key.ts`, which `morph/keys.ts` re-exports, and the build then passed its reproducibility check.
  - resolved: Removing the second attribute left a `live:key` twice on the tags that already carried a `live_key`-filtered key beside a plain one (list group, toast, live feed, datatable rows), which the checker refused as html_syntax (114 errors over five gallery views). The plain duplicate is gone; each keeps the filtered key the loop-key rule requires.
  - resolved: The form gallery's save form had no browser case, as the backlog record said, and writing one found a runtime defect no case had reached: a model binding group compared each control's read against a nullish fallback of the first, so an empty number input, whose read is null, mismatched itself; the submit's model sampling threw `model_control_invalid`, the action was never scheduled, and with `.prevent` added the browser still submitted natively because the runtime's listener never ran. The group now compares against the first read itself; `tests/model-form-groups.test.ts` pins the null, agreeing, disagreeing and disabled cases; Live spec 03 records it; the browser case fills the required controls and asserts a 200 action and no navigation.
  - resolved: The search input component wrote `live:model.debounce.300ms`, which the runtime's timing table rejects; it now writes 250 ms. The checker accepting 300 ms is a separate grammar mismatch outside this commitment, recorded in the backlog with its evidence.
  - resolved: The data-display browser case read the reordered item's key from `data-suprnova-live-key`; it reads `live:key`, the attribute the template now writes.
  - resolved: The first stale check after the build failed two groups. The dogfood document tests still asserted the doubled key markup and the 300 ms search debounce; they assert `live:key` written once and 250 ms (18 of 18 pass). The Live gate stopped at its formatting step on `src/signals/lifecycle.ts`, one of the key readers edited and never reformatted; formatted, the reproducible build still matches the committed bundles (failing receipts `20260916T134808748Z-1327171` and `20260916T134813331Z-1327171`).

## Build review, 2026-09-16

### Attacked: contradictions

- "Reads `live:key` wherever it resolves a stable key" against the runtime:
  a search for the engine attribute name under `browser/src` finds it only
  in `directives/key.ts`; every former reader calls `stableKeyOf`, and the
  transition resolver queries `KEYED_SELECTOR`, which matches both
  spellings.
- "Both present with different values fails morph validation": identity
  planning calls `keyConflict` before reading a key and fails
  `morph_identity_key_conflict`, which preflight surfaces as a
  `MorphPreflightError`; `stableKeyOf` answers `null` for a conflicting pair
  so no later reader can pick one side.
- "The engine spelling stays" against the engine: nested island roots and
  the documents the reference host renders still carry
  `data-suprnova-live-key`, and the browser unit suite's morph fixtures,
  written with it, pass unchanged.

### Falsifiers, each demonstrated

- A `live:key`-only `live:preserve.self` disclosure keeps its open state and
  its control across a compatible morph (`live-key-identity.test.ts`, first
  case); the same fixture against the pre-change runtime has no control
  under that key, which is the recorded defect.
- A pair with different values is refused with the named detail (fourth
  case); an agreeing pair and each spelling alone read one key (second,
  third and fifth cases).
- The checker still proves a `live:key` scope and refuses a duplicate
  (`checker_regressions`).

### Limits recorded

- The engine spelling is still read, so documents rendered before this
  change keep their identity; an application that writes both spellings
  with different values now fails its morph instead of morphing under one.

## Mechanism demonstrations

Receipts on `0d0d3ce8`, from the stale check after the needle fix (process 1447551, log `~/workspace2/scratchpads/live-key/check-stale-3.log`), 31 requirements passing:

- LIVE-024, `live-key-vocabulary`: `.cairn/evidence/runs/20260916T140101062Z-1447551`, the vitest fixture and the checker regression, exit 0.
- The Live gate group (LIVE-010, LIVE-011, LIVE-012, UI-008, UI-012, OVL-005, FDB-004, NAV-003): `.cairn/evidence/runs/20260916T140113470Z-1447551`, pass, the browser suites on chromium, firefox and webkit with the rebuilt bundles.
- The dogfood group (UI-006, UI-015, FORM-004, FDB-003, DATA-005, FORM-005, NAV-005): `.cairn/evidence/runs/20260916T140107692Z-1447551`, pass.
- Every other stale requirement: pass, receipts under `.cairn/evidence/runs/` from process 1447551, committed at `0d0d3ce8`.

