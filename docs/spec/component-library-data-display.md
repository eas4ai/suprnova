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
Falsifier: a shipped layout primitive reorders reading or focus order at
a breakpoint, or wraps content in an unlabeled generic element by default.
Mechanism: `.cairn/mechanisms/ui-live-check`; one Playwright case
asserting tab order across breakpoints.

[DATA-002] Badge, avatar, and stat components MUST carry text or a
programmatic label for every status. A badge, avatar, or stat component
MUST NOT convey status by color or image alone.
Falsifier: a shipped status variant has no text and no `aria-label`.
Mechanism: `.cairn/mechanisms/ui-live-check`.

[DATA-003] Repeated items in a list, feed, or table MUST carry stable
domain keys through `live:key`.
Falsifier: a shipped collection renders items without keys and a reorder
moves focus or local state to another item.
Mechanism: the morph fixtures under `crates/suprnova-live/browser/tests/`
extended with the library's collection markup.

## Behavioral tier

[DATA-004] The chart MUST render its marks on the server as SVG through
`charts-rs`, with a textual summary or data table alternative in the
canonical document, updated through normal re-render or streams. The
library MUST NOT ship a client charting runtime.
Falsifier: a chart's marks are produced by browser script, a chart lacks
a textual alternative, or a charting library appears in the browser
source.
Mechanism: a grep over the shipped views and browser source; the dogfood
document test asserts the SVG is present in a plain GET.
Reading: Live spec 25's revision of 2026-09-13 records this; `charts-rs`
(Apache-2.0, SVG with the default build, 22 chart types) enters the
workspace as a dependency of the library crate and passes the audit,
feature matrix, and license inventory like any other.

[DATA-005] The datatable MUST use native table semantics. The datatable
MUST expose sort, filter, and page state through `#[url]` fields as
shareable URLs. The datatable MUST mount as one island per table.
Falsifier: a table renders as generic elements, a sort is not reflected
in the URL, or a row mounts its own island.
Mechanism: an `app/tests/` end-to-end case through `handle_request`;
`.cairn/mechanisms/ui-live-check`.
