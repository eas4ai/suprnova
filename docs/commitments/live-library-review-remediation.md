# What the review of Live and the library found is fixed

Slug: live-library-review-remediation
Requirements: FORM-008, FORM-009, FORM-010, FORM-011, FORM-012, FDB-007, NAV-007, OVL-007, DATA-006, UI-020, UI-021, UI-022, UI-023, UI-024, LIVE-031, LIVE-032, LIVE-033, LIVE-034, LIVE-035, LIVE-036
Promoted from: the-combobox-freezes-the-page-when-the-typed-text-matches-no-option
Rests on: FORM-001, UI-017, UI-018, LIVE-024, LIVE-025

## Goal

The Live runtime and the shipped component library hold up under the
adversarial review of 2026-09-17 that preceded the 2.1.0 release: no
component can freeze or silently break the page, a form built from the
library shows and keeps the island's state, a value the server cannot
decode is refused visibly, the checker proves only what the runtime
accepts, and `live:add` and the asset route are safe to deploy. Promoted
from the backlog on the developer's direction, in conversation, 2026-09-17:
"create a commitment for all of these items that require remediation";
the decision is
`docs/decisions/the-review-of-live-and-the-component-library-is-remediated-in-one-commitment.md`.
All eighteen backlog items the review captured carry `Promoted to:
live-library-review-remediation`.

## Deliverables

- The combobox stays responsive for a query that matches no option, shows
  every option the server rendered for the input's text, and hides a
  listbox rendered for an older query (FORM-011, FORM-012, FORM-008), with
  cases that drive the shipped element in Chromium, Firefox, and WebKit.
- The form-family macros render the island's current value, checked state,
  and selected state; a radio or checkbox group keeps an unsent selection
  across a re-render; the runtime proposes a checkbox group as the list of
  checked values (FORM-009, FORM-010, LIVE-032), proved on the dogfood
  form gallery with an initial value, an untouched submit, a reset, and a
  group selection.
- An undecodable model proposal answers a field validation error and does
  not run the action (LIVE-031), with a framework test through
  `handle_request`.
- The checker, the `live_key` filter, and the runtime accept one key
  alphabet; the checker and the runtime agree on island element ids; a
  shipped filter turns any value into an accepted stable key and the
  manual states the `live_key` failure; a macro loop variable that shadows
  a parameter is checked as the loop's binding (LIVE-033 to LIVE-036),
  each with a regression.
- The toast region holds a toast's timer while the pointer is over any
  part of it or focus is inside it; nested local tabs act only on their
  own tabs and panels; the tooltip bubble stays visible under the pointer;
  every library custom element keeps one set of listeners across a morph
  move; the select indicator is visible in both schemes (FDB-007, NAV-007,
  OVL-007, UI-020, UI-024), each with a browser case.
- Vendored component assets are served to an application started outside
  its project directory, or the application refuses to start (UI-021).
- `live:add` replaces an unedited older library file and refuses a
  third-party file that is a symbolic link or resolves outside its
  directory (UI-022, UI-023), with CLI tests.
- `render_chart` returns an error for any finite series (DATA-006).
- The vendored copies under `app/templates/suprnova-ui/` match the shipped
  components; the manual chapter, its six mirrors, and the changelog in
  seven locales record the fixes.

## Out of this commitment

Dismissing the tooltip without moving the pointer or focus needs script,
which OVL-002's Agreed text excludes; it waits in `.cairn/next-iteration/`
for a specification phase.

## Records

- Requirements: `docs/spec/component-library-forms.md` (FORM-008 to
  FORM-012), `docs/spec/component-library-feedback.md` (FDB-007),
  `docs/spec/component-library-navigation.md` (NAV-007),
  `docs/spec/component-library-overlays.md` (OVL-007),
  `docs/spec/component-library-data-display.md` (DATA-006),
  `docs/spec/component-library-foundations.md` (UI-020 to UI-024),
  `docs/spec/live.md` (LIVE-031 to LIVE-036).
- Mechanism: `.cairn/mechanisms/live-library-review-remediation`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-library-review-remediation.md`.

## Done when

The mechanism passes on a committed tree, the Live gate and the repository
gate pass, the review is recorded, and `cairn wake` says Done.
