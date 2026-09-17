# Review - live-model-render-baseline

commitment: live-model-render-baseline
commit: 3c868f88
examined:
  - LIVE-037 against the built tree: `ModelState.settle`, `ModelTimingCoordinator.waiting`, `ModelFormRuntime.readRendered` and `settleRender`, and the `reconcile` port in `crates/suprnova-live/browser/src/islands/discovery.ts`, which reads each bound field's controls after the morph, lets continuity restore local edits, then settles the field.
  - `tests/model-render-baseline.test.ts`: the refused value typed again after a render, the accepted value against a restored local edit, the in-flight field and the debounced edit; each rule failing under its own mutation (settle as a no-op, the in-flight skip removed, the waiting skip removed).
  - `e2e/app-dogfood-forms.spec.ts`, the reset case without its workaround: clearing the seat count, pressing Reset and clearing it again sends the update and shows the error, on chromium, firefox and webkit, twice each; it times out on the runtime before the change.
  - The whole browser unit suite (901), ESLint, tsc, Prettier, the reproducible build with recomputed integrity pins, the correctness-delay scanner, the 2.0.2 changelog in seven locales and the translation lock.
findings:
  - resolved: `dirty` compared a field's proposal with the value its controls held at mount for the island's whole life, because `ModelState.reconcile` had no caller; the settle step now moves the accepted value to what each applied render gave the controls, which is what Live spec 11's dirty rule names, and a restored local edit stays dirty.
  - resolved: The first check found `src/models/forms.ts` unformatted in the Live gate's Prettier step; formatted, with the build unchanged.

## Build review, 2026-09-17

### Attacked: contradictions

- "The render is the baseline" against "a response cannot overwrite a newer
  unsent local edit" (Live spec 11): a field with an edit in flight or still
  waiting on its timing is skipped, and a local edit continuity restored is
  what the controls show, so the baseline is that edit, not the server's value.
- "Without counting an edit" against response ordering: `settle` never
  touches the edit sequence, so a late response still compares against the
  sequence of the edit it answers.
- A control the morph removed: only connected bindings are read and settled.

### Falsifiers, each demonstrated

- LIVE-037: on the dogfood form gallery, clearing the seat count, pressing
  Reset and clearing it again sends the update, which the runtime before the
  change never sent.

### Limits recorded

- A deferred binding (`.action` or `.submit` timing) samples its controls when
  the action runs, so its baseline between renders matters only to `dirty`.

## Mechanism demonstrations

Receipts on `3c868f88`, from the stale check (process 238418 found the Prettier step, process 962644 passed), committed at `3166159b`:

- LIVE-037, `live-model-render-baseline`: `.cairn/evidence/runs/20260917T171556171Z-962644`, the unit cases and the dogfood case, exit 0.
- The Live gate group: `.cairn/evidence/runs/20260917T171622954Z-962644`, pass.
- The remediation group, whose dogfood form cases include the rewritten reset case: `.cairn/evidence/runs/20260917T173047394Z-962644`, all twenty requirements passing.
- `ui-tokens`, `ui-light-dom`: pass, receipts from process 962644.
