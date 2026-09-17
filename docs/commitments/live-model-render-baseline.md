# A render that changes a control is the baseline for its next edit

Slug: live-model-render-baseline
Requirements: LIVE-037
Rests on: LIVE-031, FORM-009

## Goal

A model edit reaches the island whenever it differs from what the control
shows after the last render, so a control and its island never sit out of
step without an error: after a refused value and a render that replaces it,
the same value typed again is sent and refused again. Promoted from the
backlog on the developer's `ok` to escalation `live-031`, 2026-09-17; the
decision is `docs/decisions/a-render-that-changes-a-bound-control-becomes-the-baseline-its-next-edit-is-compared-with.md`.

## Deliverables

- After the island applies a render, the runtime takes the value each bound
  control then holds as that field's baseline, except for a field with an
  edit in flight or not yet sent, so the next edit is compared with what the
  user sees; vitest cases for the reconciled field, the in-flight field and
  the unsent edit, failing on the current runtime.
- The dogfood reset case drops its workaround: clearing the seat count,
  pressing Reset and clearing it again sends the update and shows the field
  error, on chromium, firefox and webkit, failing on the current runtime.
- The rebuilt runtime artifacts and their integrity pins; the 2.0.2
  changelog records the fix in seven locales.

## Records

- Requirement: `docs/spec/live.md` (LIVE-037).
- Mechanism: `.cairn/mechanisms/live-model-render-baseline`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-model-render-baseline.md`.

## Done when

The mechanism passes on a committed tree, the Live gate and the repository
gate pass, the review is recorded, and `cairn wake` says Done.
