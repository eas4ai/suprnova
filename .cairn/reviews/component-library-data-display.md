# Review - component-library-data-display

commitment: component-library-data-display
commit: bfb6ad82
examined:
  - DATA-001 to DATA-005 against the built tree, their mechanisms and the receipts recorded on it.
  - The inherited foundations, overlay, feedback, navigation, live:check, live:add and dogfood mechanisms rerun over the eleven new component directories and the data-display page.
  - The charts module in the Live crate, its bounds and its four tests, and the charts-rs dependency under the audit.
  - The datatable island's URL binding across a fresh mount from the query, a reflected action and a native submit, on chromium, firefox and webkit.
findings:
  - resolved: The datatable's sort and filter forms first submitted natively as well as through Live, because `live:submit` without the `prevent` modifier leaves the native submission alone; every click remounted the island from a GET of the form's own query and the direction never flipped. Both forms now carry `live:submit.prevent`. The form gallery's own submit shares the trait and no browser case exercises it; filed in `.cairn/backlog/` under FORM-004, not changed here.
  - resolved: Model proposals never reached the sort field until the inputs carried `live:model` and the request carried a `sync_model` operation per field, which is how the runtime and the document test now send them.
  - resolved: A `u64` mount parameter is a tagged canonical value, not a plain number, so the first mount was rejected; the mount takes a `u32` page like the navigation island and widens it into the URL-bound field.
  - resolved: The datatable view first crossed the checker's branch limit because every column's two conditions nested a direction condition; the island now holds the direction word and the direction mark is CSS on `aria-sort`, one condition per column.
  - resolved: The chart helpers were methods, which the generated view cannot call; they are free functions the template calls by module path over the island's series field.
  - resolved: The first gallery referenced an image path nothing served; the aspect image now carries an inline SVG data URI.
  - resolved: DATA-001, DATA-002, DATA-004 and DATA-005 were each declared by two mechanisms and the loop refused them (LOOP-056); every DATA requirement now has one owner, and the live:check and Live gate runs cover the family through the inherited requirements.

## Build review, 2026-09-15

### Attacked: contradictions

- DATA-001 against the card: a card needs a grouping surface and an
  actions row, and both are generic elements. The card is an article or
  section labeled by its own heading, and the actions row is a labeled
  group; the tool fails any other generic wrapper in a layout primitive.
- DATA-002 against the stat card: a trend wants an arrow. The direction is
  a word in text before the delta; color follows it and never replaces it.
- DATA-003 against the checker: every collection item is a loop key, so the
  list group and the datatable rows pass through `live_key`, and the fixture
  proves the identity plan pairs every surviving key across a reorder, an
  insert, a removal and a sort.
- DATA-004 against trust: charts-rs produces markup from numbers and the
  labels the application passes. `render_chart` bounds the series and the
  labels, refuses markup characters, and returns framework-generated
  trusted markup; the SVG lives in an `aria-hidden` box beside a summary and
  a data table, so the canonical document reads without it. No image codec
  joins the workspace: the dependency enters with default features off.
- DATA-005 against the runtime: a reflected URL omits default values, and
  the document mounts the table from that query, so an action may start
  from a field that hydrated empty. Every action normalizes the applied
  sort, direction and page first, and the browser case proves a fresh mount
  from `?sort=amount` flips to descending on the next sort.

### Falsifiers, each demonstrated

- DATA-001, DATA-002, DATA-004: `.cairn/tools/ui-data-display.mjs` fails on
  a scratch copy whose scroll area loses `tabindex`, whose card wraps its
  content in an unlabeled div, whose badge drops its text, whose stat loses
  the text direction, whose chart loses its data table or ships a script,
  whose stylesheet names a charting library, and whose gallery shows one
  trend direction (eight mutations run on 2026-09-15).
- DATA-003: `tests/collection-continuity.test.ts` asserts the identity plan
  across a reorder, an insert with a removal, a sort and a filter.
- DATA-005: the dogfood document test mounts from the shared query, clamps
  `?page=99`, and follows sort, sort again, filter and next page through the
  reflected URL intent; the browser case does the same through the runtime
  and reloads onto the same view.

### Limits recorded

- The datatable sorts by one column with a submit per header; multi-column
  sort and row selection stay with the interactive data grid pattern spec 25
  keeps outside the built-in set.
- `render_chart` draws bar and line charts; the other charts-rs kinds are
  reachable through the crate but not through the library's surface.
- A `live:submit` without `prevent` submits natively as well; the manual's
  datatable text shows the modifier, and the form gallery's own submit is a
  backlog item.

## Mechanism demonstrations

Receipts on the built tree: DATA-001, DATA-002 and DATA-004 from the
data-display tool and DATA-003 from the collection morph fixture with the
inherited UI, OVL, FDB and NAV tool receipts (`20260915T130625Z` to
`20260915T130634Z`), DATA-005 with FDB-003, UI-006, UI-015 and FORM-004
from the dogfood tests (`20260915T130815Z`), the live:check receipts and
UI-017 (`20260915T131007Z`, `20260915T131019Z`), UI-008, UI-012, OVL-005,
FDB-004, NAV-003 and LIVE-010 to LIVE-012 from the Live gate
(`20260915T132827Z`), and UI-019 (`20260915T132855Z`).
