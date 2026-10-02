# Component library - data display and layout

Status: Draft
Prefix: DATA

Refines Live spec 25
(`crates/suprnova-live/docs/specs/suprnova-live/25-data-display-and-layout-components.md`):
layout primitives, cards and statistics, lists and feeds, tables, badges
and avatars, media and visualization containers. Inherits every Agreed
block of `component-library-foundations.md`.

Components in complexity order. Presentational: separator, scroll area,
image with aspect ratio, card, badge, avatar and group, list group,
description list, stat card. Behavioral: chart (server-rendered SVG
through `charts-rs`), datatable (sortable, filterable, paginated) - the
last component the library builds, by the developer's 2026-09-12 ruling.
No custom-element tier in this family. The interactive data grid Live
spec 25 describes as a separate opt-in pattern is not in the built-in
set; neither is any client-side chart - the developer ruled on
2026-09-13 (10:54) that charts-rs is the default and the only built-in,
and the bring-your-own visualization container stays a documented pattern
with one manual example.

## Presentational tier

[DATA-001] Every layout primitive MUST preserve logical DOM order under
visual reflow. A layout primitive MUST accept semantic elements rather
than emitting anonymous wrapper depth by default.
Falsifier: a shipped layout primitive reorders reading or focus order at a breakpoint, or wraps content in an unlabeled generic element by default.
Mechanism: `ui-live-check`; one Playwright case asserting tab order across breakpoints.
Status: Agreed 2026-09-15

[DATA-002] Badge, avatar, and stat components MUST carry text or a
programmatic label for every status. A badge, avatar, or stat component
MUST NOT convey status by color or image alone.
Falsifier: a shipped status variant has no text and no `aria-label`.
Mechanism: `ui-live-check`.
Status: Agreed 2026-09-15

[DATA-003] Repeated items in a list, feed, or table MUST carry stable
domain keys through `live:key`.
Falsifier: a shipped collection renders items without keys and a reorder moves focus or local state to another item.
Mechanism: the morph fixtures under `crates/suprnova-live/browser/tests/` extended with the library's collection markup.
Status: Agreed 2026-09-15

## Behavioral tier

[DATA-004] The chart MUST render its marks on the server as SVG through
`charts-rs`, with a textual summary or data table alternative in the
canonical document, updated through normal re-render or streams. The
library MUST NOT ship a client charting runtime.
Falsifier: a chart's marks are produced by browser script, a chart lacks a textual alternative, or a charting library appears in the browser source.
Mechanism: a grep over the shipped views and browser source; the dogfood document test asserts the SVG is present in a plain GET.
Rationale: Reading: Live spec 25's revision of 2026-09-13 records this; `charts-rs` (Apache-2.0, SVG with the default build, 22 chart types) enters the workspace as a dependency of the library crate and passes the audit, feature matrix, and license inventory like any other.
Status: Agreed 2026-09-15

[DATA-005] The datatable MUST use native table semantics. The datatable
MUST expose sort, filter, and page state through `#[url]` fields as
shareable URLs. The datatable MUST mount as one island per table.
Falsifier: a table renders as generic elements, a sort is not reflected in the URL, or a row mounts its own island.
Mechanism: an `app/tests/` end-to-end case through `handle_request`; `ui-live-check`.
Status: Agreed 2026-09-15

[DATA-006] The chart renderer MUST return an error instead of panicking
for any series of finite values.
Falsifier: a call to `render_chart` with a value of 1e12 panics.
Mechanism: `live-library-review-remediation`.
Rationale: Evidence: charts-rs 1.0.0 overflows in `src/charts/util.rs` for values from 1e12, and `crates/suprnova-live/src/view/charts.rs` checks only that each value is finite. Refines: DATA-004; the house rule that public-surface code returns a Result and does not panic. Agreed by promotion `the-review-of-live-and-the-component-library-is-remediated-in-one-commitment`.
Status: Agreed 2026-09-17

[DATA-007] The chart MUST take its colors and font from `--sn-` tokens in
both color schemes. Its SVG MUST have a transparent background, and its
text, axes, grid lines and each series MUST carry a class that `chart.css`
colors from a `--sn-` token; series take their colors in order from a
palette of chart tokens, defined for light and dark, and wrap around when a
chart has more series than the palette. The rendered SVG MUST NOT carry a
literal color or font family.
Falsifier: a rendered bar or line chart's SVG contains a hex, `rgb()` or named color, or a font-family name, in an attribute or a `style`; a chart's text, axis, grid or series mark carries no class; or `chart.css` colors a chart class with anything but a `var(--sn-...)` reference.
Mechanism: `data-chart-tokens`.
Rationale: Backlog item `chart-follows-the-tokens`: the charts-rs built-in light theme writes a white background, `rgb(70, 70, 70)` text, a fixed series palette and the Roboto font into the markup, so the chart shows as a white box in a dark document and ignores the theme, while every other shipped component takes its visual values from the tokens (UI-004 states that rule for stylesheets). Refines: DATA-004.
Status: Agreed 2026-10-01
