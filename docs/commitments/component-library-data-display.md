# Component library data display

Slug: component-library-data-display
Requirements: DATA-001, DATA-002, DATA-003, DATA-004, DATA-005
Rests on: UI-001, UI-002, UI-003, UI-006, UI-009, UI-010, UI-012, UI-013, UI-014, UI-017, LIVE-008, LIVE-009, LIVE-011, NAV-003

## Goal

Suprnova ships the data display and layout family of its Live component
library on the foundations, the overlays, the feedback family and the
navigation family: separator, scroll area, image with aspect ratio, card,
badge, avatar and avatar group, list group, description list and stat
card as presentational components; the chart, rendered on the server as
SVG through `charts-rs` with a textual alternative; and the datatable,
the last component the library builds by the developer's 2026-09-12
ruling, on native table semantics with its sort, filter and page state
bound to shareable URLs through `#[url]` fields and one island per table.

Drafted 2026-09-15 after `cairn wake` reported
component-library-feedback-and-navigation Done and the roadmap named this
item next; the developer ruled on 2026-09-14 at 23:35 that every
remaining commitment is agreed, and the five requirements are recorded
as one set through escalation `data-001`.

Not in this commitment: the interactive data grid, which Live spec 25
keeps as a separate opt-in pattern outside the built-in set; any
client-side chart, by the developer's 2026-09-13 ruling; the code block,
prose and media containers, which the roadmap does not name.

## Deliverables

- Eleven component directories under `crates/suprnova-live/components/`
  (separator, scroll-area, aspect-image, card, badge, avatar, list-group,
  description-list, stat-card, chart, datatable), each with a manifest,
  an Askama macro view, CSS under the `suprnova-ui` layer with `--sn-`
  tokens only, and no JavaScript: the scroll area is a focusable region
  with native scrolling, the datatable's controls are anchors and Live
  buttons, and the chart is SVG in the canonical document.
- The chart macro takes a `charts-rs` rendering produced by the island
  and a data table alternative rendered beside it in a disclosure;
  `charts-rs` enters the Live crate as a dependency without its raster
  features, so no image codec joins the workspace.
- The datatable island exposes sort, filter and page through `#[url]`
  fields in reflect mode, renders every row keyed through `live_key`,
  and mounts once per table; its header cells carry `scope`, its sort
  links carry `aria-sort` and real URLs, and its caption names the
  result count.
- The dogfood gallery gains a data-display page mounting every shipped
  component, with a chart island and a datatable island over a seeded
  collection.
- Manual: the component library section of `manual/live.md` lists the
  family, the datatable URL binding and the chart alternative; the six
  mirrors are re-stamped under the translation lock; the 2.0.2 changelog
  section gains the entry.
- Live spec 25 gains a dated "Decisions and revisions" entry for any
  ruling the build needs; iteration 010 records the scope.

## Records

- Requirements: `docs/spec/component-library-data-display.md` (DATA), on
  the Agreed foundations, overlays, feedback and navigation records.
- Mechanisms, declared after the agreement: a data-display tool under
  `.cairn/tools/` reporting per requirement over the shipped views and
  the gallery (DATA-001, DATA-002, DATA-004), `ui-live-check` (DATA-001,
  DATA-002, DATA-005), `live-gate` (DATA-001 through the Playwright
  matrix), a morph fixture mechanism over the collection markup
  (DATA-003), `ui-dogfood-tests` (DATA-004, DATA-005), and the inherited
  `ui-light-dom`, `ui-elements`, `ui-islands`, `ui-tokens`, `ui-live-add`
  and `ui-framework-tests` runs over the new directories.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-data-display.md`.

## Tests that prove it

- No shipped layout primitive emits a generic wrapper by default or
  reorders content with CSS `order`; Playwright on chromium, firefox and
  webkit tabs through the gallery at a narrow and a wide viewport and
  records the same focus order (DATA-001).
- Every badge, avatar and stat variant in a shipped view carries text or
  an `aria-label`, and the stat's trend has a text direction cue
  (DATA-002).
- The morph fixture reorders, inserts and removes keyed list-group items
  and datatable rows and keeps every surviving node (DATA-003).
- No charting script appears in the browser source or the shipped views;
  the dogfood document test finds the chart SVG and its data table in a
  plain GET (DATA-004).
- The datatable document is a `table` with `caption`, `thead`, `th
  scope`; a sort link carries the reflected URL; the dogfood test mounts
  the table from `?sort=` and `?page=` and the document carries exactly
  one datatable island (DATA-005).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
