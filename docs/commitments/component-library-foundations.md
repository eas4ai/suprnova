# Component library foundations

Slug: component-library-foundations
Requirements: UI-001, UI-002, UI-003, UI-004, UI-005, UI-006, UI-007, UI-008, UI-009, UI-010, UI-011, UI-012, UI-013, UI-014, UI-015, FORM-001, FORM-002, FORM-003, FORM-004
Rests on: LIVE-003, LIVE-004, LIVE-005, LIVE-006, LIVE-008, LIVE-009, LIVE-010, LIVE-011, LIVE-013, LIVE-014

## Goal

Suprnova ships the foundations of its official Live component library:
a token stylesheet with a base layer, state styling keyed to proven
accessibility attributes, library assets delivered under the runtime's
artifact contract with baseline rows, explicit registration under
reserved namespaces, and the form family's presentational tier - every
shipped view checker-clean, in light DOM, on the three qualified engines.

Drafted 2026-09-13 from the developer's 2026-09-12 rulings, his 09:57
registration ruling, and Live specs 20 and 21. Not Agreed: the rulings
listed in `docs/spec/component-library-foundations.md` gate the
requirements above, and every UI and FORM requirement is Draft until the
developer confirms its text and falsifier as one set.

## Decisions to record before code

- Distribution model (Consequential: it shapes every later commitment):
  vendored presentational macros versus an in-tree crate; recorded with
  `cairn decide` once the developer rules.
- Styling build path (Consequential): Tailwind CSS 4 utilities per Live
  spec 20 versus token rules only; the developer's ruling, and if it
  revises spec 20, the dated entry in that spec's "Decisions and
  revisions" section precedes the code (LIVE-014).
- Explicit registration and reserved namespaces (Consequential, ruled
  2026-09-13 09:57): recorded in `docs/decisions/`.
- Naming (Judged): the reserved component prefix and the CLI verb.

## Deliverables

- The token stylesheet and base layer as reviewed artifacts with baseline
  rows, inside the `suprnova-ui` cascade layer with `--sn-` tokens; light
  and dark from day one; motion tokens with `prefers-reduced-motion`
  respected.
- The form family's presentational tier: field, label, input, textarea,
  number input, slider, search input, password input with reveal,
  checkbox and group, radio group, switch, select, button and
  link-button, button group, fieldset, form actions bar, validation
  summary, file input.
- The reserved namespaces enforced: `suprnova.` names, the `suprnova-ui/`
  template root, `sn-` tags, and the opt-in asset role on
  `LiveBootstrapOptions`.
- A dogfood view under `app/templates/live/` that mounts every shipped
  component, so `live:check` and the document tests exercise the real
  set.
- The Live spec revisions the rulings require, each a dated "Decisions
  and revisions" entry, and `iterations/007.md` in Live's spec set if the
  developer keeps Live's contract convention.
- `manual/live.md` gains the library section; the six mirrors are
  re-stamped under the translation lock.

## Records

- Requirements: `docs/spec/component-library-foundations.md` (UI),
  `docs/spec/component-library-forms.md` (FORM), and the Observed base in
  `docs/spec/live.md` (LIVE).
- Mechanisms: `.cairn/mechanisms/ui-tokens`, `ui-live-check`,
  `ui-light-dom`, `live-gate`, `live-contracts`, `spec-lint`. The token
  mechanism reads `crates/suprnova-live/browser/src/styles/suprnova-ui.css`,
  the proposed location of the shipped stylesheet; it fails honestly until
  the file exists, and the distribution ruling may move it.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-foundations.md`.

## Tests that prove it

- The stylesheet check fails when a token role is missing, when a base
  element has no rule, when a state is selected by class alone, when a
  rule sits outside the `suprnova-ui` layer, or when a token lacks the
  `--sn-` prefix; passes on the shipped stylesheet (UI-001, UI-002,
  UI-003, UI-013). Demonstrated on disposable fixtures; see the review.
- The Live gate's tracked artifact parity fails on an edited library
  artifact and the baseline check fails on a missing row (UI-004,
  LIVE-010).
- `suprnova live:check` over the dogfood application, no
  `--allow-unproved`, with every library component mounted (UI-005,
  UI-010, UI-012, FORM-001, FORM-002, FORM-003).
- A registry unit test rejects a `suprnova.` name from outside the
  library (UI-011).
- A grep for `attachShadow` and for unprefixed library tags in library
  source is empty; a Playwright case boots a library-free document and
  finds no `sn-` definition (UI-006, UI-014).
- One browserless harness case and one Playwright case per
  custom-element enhancement that carries a value (UI-007).
- A framework test shows a document without the opt-in serves no library
  asset (UI-015).
- The Playwright matrix on chromium, firefox, webkit (UI-008, LIVE-011).
- Snapshot tests show no password or one-time-code value (FORM-004).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
