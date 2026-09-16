# The checker proves what the runtime accepts

Slug: checker-proves-runtime-accepts
Requirements: LIVE-025, LIVE-026, LIVE-027
Rests on: LIVE-008, UI-009

## Goal

`live:check` proves a component only after checking every element its view
renders, and what it proves is what the browser runtime accepts: a
declared debounce always has a template modifier both sides accept, and
error feedback may target an action the way the runtime resolves it.
Promoted from the backlog on the developer's `ok` to escalation
`live-024`, and widened to the checker's false proof on the `ok` to
escalation `live-008`, 2026-09-16.

## Deliverables

- The checker renders an empty call block as one empty caller branch and
  fails a component whose view renders no branch; a regression proves an
  undeclared action after an empty call is reported.
- `#[model(debounce = N)]` and `BindingTiming::debounce` accept only the
  grammar's durations, with a compile-fail UI test and the metadata test
  updated; the dogfood galleries declare 250 ms.
- The checker accepts a `live:error` target naming a declared field or an
  action of the component or an ancestor, with a regression.
- Every error the fixed checker exposes in the library and the dogfood
  application is fixed: the form gallery declares the upload field its
  file input binds, with its policy and upload abilities.
- Live specs 03, 11 and 19 record the rules; `manual/live.md` names the
  debounce durations with the six mirrors re-stamped; the 2.0.2 changelog
  records the fixes in seven locales.

## Records

- Requirements: `docs/spec/live.md` (LIVE-025, LIVE-026, LIVE-027).
- Mechanism: `.cairn/mechanisms/checker-soundness`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/checker-proves-runtime-accepts.md`.

## Done when

The mechanism passes on a committed tree, `live:check` proves every
dogfood component with the fixed checker, the Live gate passes, the review
is recorded, and `cairn wake` says Done.
