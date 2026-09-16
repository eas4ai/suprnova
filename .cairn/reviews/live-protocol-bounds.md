# Review - live-protocol-bounds

commitment: live-protocol-bounds
commit: 7b4918d7
examined:
  - LIVE-028 against the built tree: the named bounds in `crates/suprnova-live/browser/src/protocol.ts` for proposals, operations, action arguments, validation entries, events, effects and extensions in both protocol versions, beside `framework/src/live/runtime.rs`'s `ProtocolLimits`; `tests/protocol-bounds.test.ts` building a ten-field and a 127-field submit and refusing 128 fields plus the action, and a response with seventeen validation entries and nine events accepted and 129 refused, from the reviewed v2 fixtures.
  - LIVE-029: `observe_submit_form` in `crates/suprnova-live/src/checker/html.rs` and `submit_form_proposals_are_bounded_by_one_request`, one report at 128 fields, none at 127, none for controls outside the form.
  - LIVE-030: `refusedRequestDiagnostic` in the island transport, classifying only bound refusals as `resource_limit` / `resource_exhausted`; the existing catch still finishes the action rejected, whose feedback state is error.
  - The dogfood form gallery's save form with its ten model controls, submitting on chromium, firefox and webkit; the whole browser unit suite (893), the Live crate suite (941), the dogfood document tests, `live:check`, ESLint, clippy and the reproducible build with recomputed integrity pins.
findings:
  - resolved: The promotion recommended that a submit fit the eight-proposal bound or fail visibly. The build found the bound only in the browser's validators, below the framework server's 128 and the browser scheduler's own 128, with no specification behind it; aligning the browser to the server, recorded as a Consequential decision, keeps the checker and the visible failure for the real bound.
  - resolved: The browser's intent also capped operations at 32, a third count below the server, so a submit of more than 31 fields would have stopped before the validators; it is 128, and the builder test now checks the intent limit where the validators' bound can no longer be reached.
  - resolved: The first checker regression could not see its own report: the fixture's fields are undeclared in the test registry, and their unknown-model reports filled the default 64-diagnostic ceiling before the 128th control; the test runs with room for every report. In an application the fields are declared, and a filled ceiling still fails the check.

## Build review, 2026-09-16

### Attacked: contradictions

- "Admit the counts the framework's protocol limits admit" against the
  canonical parser: its own ceiling is 2048 entries per message, which a
  message at 128 of each dimension stays under.
- "127 fields" against the operations bound: a submit sends one sync
  operation per field and the invoked action, so 128 operations carry 127
  fields; the checker and the vitest request case use the same arithmetic.
- "Fails visibly" against the transport: the refusal never leaves the
  browser, the action finishes rejected, and `live:error` for the action
  shows; only the diagnostic's code changed.

### Falsifiers, each demonstrated

- LIVE-028: a nine-proposal request and a seventeen-entry response were
  refused before the change, the failing Live gate receipt of the previous
  commitment (`20260916T161220094Z-2630644`, firefox), and pass now.
- LIVE-029: 128 distinct fields in a `live:submit` form report once.
- LIVE-030: a bound refusal classifies as `resource_limit`; a network
  failure does not.

### Limits recorded

- Child deliveries stay bounded at 8 in the browser; the framework has no
  configured limit for them to align with.
- A form whose controls join through a `form` attribute from outside the
  element is not counted by the checker; the runtime still refuses such a
  submit past the bound, visibly.

## Mechanism demonstrations

Receipts on `7b4918d7`, from the stale check (process 4056710, log `~/workspace2/scratchpads/live-key/check-bounds.log`), 35 requirements passing:

- LIVE-028, LIVE-029, LIVE-030, `live-protocol-bounds`: `.cairn/evidence/runs/20260916T192151530Z-4056710`, the vitest bounds file and the checker regression, exit 0.
- The Live gate group: `.cairn/evidence/runs/20260916T192436771Z-4056710`, pass on chromium, firefox and webkit with the rebuilt bundles.
- The dogfood group: `.cairn/evidence/runs/20260916T192158065Z-4056710`; the `live:check` group: `.cairn/evidence/runs/20260916T194642680Z-4056710`; both pass.
- Every other stale requirement: pass, receipts from process 4056710 committed at `7b4918d7`.
