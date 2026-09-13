# Component library foundations

Slug: component-library-foundations
Requirements: UI-001, UI-002, UI-003, UI-004, UI-005, UI-006, UI-007, UI-008, UI-009, UI-010, UI-011, UI-012, UI-013, UI-014, UI-015, UI-016, UI-017, UI-018, UI-019, FORM-001, FORM-002, FORM-003, FORM-004
Rests on: LIVE-003, LIVE-004, LIVE-005, LIVE-006, LIVE-008, LIVE-009, LIVE-010, LIVE-011, LIVE-013, LIVE-014

## Goal

Suprnova ships the foundations of its official Live component library:
a token stylesheet with a base layer, state styling keyed to proven
accessibility attributes, library assets delivered under the runtime's
artifact contract with baseline rows, explicit registration under
reserved namespaces, and the form family's presentational tier - every
shipped view checker-clean, in light DOM, on the three qualified engines.

Drafted 2026-09-13 from the developer's 2026-09-12 rulings, his 09:57
registration ruling, and Live specs 20 and 21; the four remaining rulings
were walked one by one between 10:20 and 10:54 and every requirement
above was confirmed as one set at 11:08. Agreed.

## Decisions recorded (all Consequential, all ruled 2026-09-13)

Each is a record under `docs/decisions/` and a dated entry in the owning
Live spec's "Decisions and revisions" section (LIVE-014):

- Explicit registration and reserved namespaces (09:57).
- Styling: token rules under the `suprnova-ui` layer with a Tailwind 4
  `@theme` preset (10:22).
- Distribution: crate-owned behavior, `live:add`-vendored views, macros,
  CSS, and JavaScript from a JSON manifest (10:23).
- Custom elements: light-DOM, form-associated, per-component vendored
  definitions on a reviewed helper (10:36).
- Headless by construction: no inline styles, tokens only, skin removable
  (10:36-10:37).
- Chart: one built-in, server-rendered through `charts-rs`; no client
  charting runtime (10:54).

Naming strings (`suprnova.`, `suprnova-ui/`, `--sn-`, `sn-`, `live:add`)
stand as written in the spec unless the developer changes them.

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
- `live:add` reading a JSON manifest per component, installing the
  component directory (view, CSS, JavaScript) without overwriting an
  edited file, and accepting a third-party manifest.
- The Tailwind CSS 4 `@theme` preset mapping to the `--sn-` tokens,
  documented and tested against a pinned range.
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
  element has no rule or a rule sits outside the `suprnova-ui` layer,
  when a state is selected by class alone, when a literal visual value
  appears, when a view carries a `style` attribute, or when the Tailwind
  preset misses a token; passes on the shipped files (UI-001, UI-002,
  UI-003, UI-004, UI-005, UI-007). Demonstrated on disposable fixtures;
  see the review.
- A browserless harness run with the base layer stripped keeps every
  behavior, accessible name, and state attribute (UI-006).
- The Live gate's tracked artifact parity fails on an edited shared base
  and the baseline check fails on a missing row (UI-008, LIVE-010).
- `suprnova live:check` over the dogfood application, no
  `--allow-unproved`, with every library component mounted (UI-009,
  UI-014, UI-016, FORM-001, FORM-002, FORM-003).
- A registry unit test rejects a `suprnova.` name from outside the
  library (UI-015).
- A CLI test installs a component twice with an edit between runs and
  the edit survives; a third-party manifest installs the same way
  (UI-017).
- A grep for `attachShadow` and for unprefixed library tags in library
  source is empty; a Playwright case boots a library-free document and
  finds no `sn-` definition (UI-010, UI-018).
- One browserless harness case and one Playwright case per
  custom-element enhancement that carries a value (UI-011).
- A framework test shows a document without the opt-in serves no shared
  base (UI-019).
- The Playwright matrix on chromium, firefox, webkit (UI-012, LIVE-011).
- Snapshot tests show no password or one-time-code value (FORM-004).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
