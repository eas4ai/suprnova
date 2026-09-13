# Component library foundations

Slug: component-library-foundations
Requirements: UI-001, UI-002, UI-003, UI-004, UI-005, UI-006, UI-007, UI-008, UI-009, UI-010, UI-011
Rests on: LIVE-003, LIVE-005, LIVE-008, LIVE-009, LIVE-010, LIVE-011, LIVE-013, LIVE-014

## Goal

Suprnova ships the foundations of its official Live component library:
a token stylesheet with a base layer, state styling keyed to proven
accessibility attributes, library assets delivered under the runtime's
artifact contract with baseline rows, and the complete form and input
family - every shipped view checker-clean, in light DOM, on the three
qualified engines.

Drafted 2026-09-13 from the developer's 2026-09-12 rulings and Live specs
20 and 21. Not Agreed: six rulings listed in `docs/spec/component-library.md`
("Rulings the developer still owes") gate the requirements above, and
UI-001 to UI-011 are Draft until the developer confirms their text and
falsifiers as one set.

## Decisions to record before code

- Distribution model (Consequential: it shapes every later commitment):
  vendored presentational macros versus an in-tree crate; the developer's
  ruling is recorded with `cairn decide` once given.
- Styling build path (Consequential): Tailwind CSS 4 utilities per Live
  spec 20 versus token rules only; the developer's ruling, and if it
  revises spec 20, the dated entry in that spec's "Decisions and
  revisions" section precedes the code (LIVE-014).
- Naming (Judged): the reserved component prefix and the CLI verb.

## Deliverables

- The token stylesheet and base layer as reviewed artifacts with baseline
  rows; light and dark from day one; motion tokens with
  `prefers-reduced-motion` respected.
- The form and input family: field, input, textarea, checkbox and group,
  radio group, switch, select, label, button and link-button, button
  group, fieldset, form actions bar, validation summary.
- A dogfood view under `app/templates/live/` that mounts every shipped
  component, so `live:check` and the document tests exercise the real
  set.
- The Live spec revisions the rulings require, each a dated "Decisions
  and revisions" entry, and `iterations/007.md` in Live's spec set if the
  developer keeps Live's contract convention.
- `manual/live.md` gains the library section; the six mirrors are
  re-stamped under the translation lock.

## Records

- Requirements: `docs/spec/component-library.md` (UI) and the Observed
  base in `docs/spec/live.md` (LIVE).
- Mechanisms: `.cairn/mechanisms/ui-tokens`, `ui-live-check`,
  `ui-light-dom`, `live-gate`, `live-contracts`, `spec-lint`. The token
  mechanism reads `crates/suprnova-live/browser/src/styles/suprnova-ui.css`,
  the proposed location of the shipped stylesheet; it fails honestly until
  the file exists, and the distribution ruling may move it.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-foundations.md`.

## Tests that prove it

- The stylesheet check fails when a token role is missing, when a base
  element has no rule, or when a state is selected by class alone; passes
  on the shipped stylesheet (UI-001, UI-002, UI-003). Safe violating
  example: a disposable copy of the stylesheet with one role deleted.
- The Live gate's tracked artifact parity fails on an edited library
  artifact and the baseline check fails on a missing row (UI-004,
  LIVE-010).
- `suprnova live:check` over the dogfood application, no
  `--allow-unproved`, with every library component mounted (UI-005,
  UI-010). Safe violating example: a disposable view referencing an
  undeclared action.
- A grep for `attachShadow` in library source is empty (UI-006).
- One browserless harness case and one Playwright case per
  custom-element enhancement that carries a value (UI-007).
- The Playwright matrix on chromium, firefox, webkit (UI-008, LIVE-011).
- Snapshot tests show no password or one-time-code value (UI-011).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
