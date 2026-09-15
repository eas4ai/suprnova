# Review - component-library-feedback-and-navigation

commitment: component-library-feedback-and-navigation
commit: cdc648b53ee59b3aa9af6fd94194b5d5ce29e294
examined:
  - FDB-001 to FDB-004, FDB-006, NAV-001 to NAV-004 and NAV-006 against the built tree, their mechanisms and the receipts recorded on it.
  - The inherited foundations, overlay, live:check, live:add and dogfood mechanisms rerun over the fourteen new component directories and the two galleries.
  - The live_key view filter and its re-export, against the checker's keyed-loop rule and the four filter tests.
  - The browser cases on chromium, firefox and webkit at the pinned Playwright version, and the Live gate that runs them.
findings:
  - resolved: The first browser run rejected every Live pagination action, because URL reflection is a protocol 2 result and the navigation island declared none; the island now declares minimum_protocol_version = 2 and the manual says so. No component changed.
  - resolved: A dismissed toast kept its hidden attribute but stayed visible, because the toast's flex display outranked the user agent's hidden rule; the stylesheet now gives a hidden toast display none. Fixed before the components were committed.
  - resolved: The load-more browser case first witnessed row identity with a browser-added attribute, which the morph removes by design when it syncs attributes from the server render; the case now witnesses with a property on the node, which only a kept node carries.
  - resolved: The navigation tool assumed each gallery import alias equals the macro name and reported three missing mounts; it now reads the alias from the import line.
  - resolved: Two hand-run three-engine passes of the app dogfood spec timed out on page loads while the machine sat at sixty percent I/O stall under the desktop indexer; every engine passed the file alone, the failing cases carried no assertion, and the Live gate then ran the same three-engine matrix on this tree and passed (receipt 20260915T044033Z). The stall was machine state, not a defect in the tree; no code changed for it.

## Build review, 2026-09-15

### Attacked: contradictions

- FDB-002 against the runtime: a spinner or skeleton must never flash on a
  fast action, and the runtime's loading target already delays 150 ms and
  holds 200 ms. The components carry no timer; they are authored hidden and
  bound through `live:loading.show`, and the tool fails a stylesheet that
  adds an animation or transition delay of its own.
- FDB-004 against FDB-001: a critical error may reach the toast region, and
  the toast region is polite by construction. The gallery's `fail` action
  renders the persistent error alert and the error toast in the same morph,
  and the browser case asserts both.
- NAV-006 against the checker: a keyed row inside a `{% for %}` loop is
  what a load-more list is, and the checker refuses a dynamic loop key
  unless it passes through `live_key`. No crate shipped that filter. It
  lands in the Live crate's view filters with the checker's exact rule,
  re-exported from the framework, and the toast macro and the feed list use
  it. Four tests cover the accepted set, the forbidden byte, the empty key
  and the bounds.
- NAV-003 against the document: the reflected query must survive a reload,
  so the navigation page mounts its island from `?page=` and clamps it; the
  document test proves `?page=99` lands on the last page.
- NAV-002 against the morph: a local tab panel is preserved so the browser
  owns `hidden`, and preservation covers only the element's own attributes,
  so server-rendered content inside a panel still morphs. The browser case
  reads the page witness inside the summary panel after a Live page change.

### Falsifiers, each demonstrated

- FDB-001, FDB-002, FDB-006: `.cairn/tools/ui-feedback.mjs` fails on a
  scratch copy whose error alert takes `role="status"`, whose spinner
  stylesheet adds an `animation-delay`, whose progress carries a value for
  indeterminate work, and whose gallery mounts only one progress kind (all
  four run on 2026-09-15; the baseline receipt records every requirement
  failing on the empty tree).
- NAV-001, NAV-002, NAV-004: `.cairn/tools/ui-navigation.mjs` fails on a
  scratch copy whose breadcrumb anchor carries `live:click`, whose tabs lose
  the route branch, whose sidebar takes `aria-current` from a script hook
  instead of a bound value, and whose load-more control becomes a link (all
  four run on 2026-09-15).
- FDB-003: the dogfood document tests render one document per reason,
  assert the no-permission reason offers no button, and assert an unknown
  reason falls back to empty.
- FDB-004 and NAV-003: `e2e/app-dogfood.spec.ts` asserts the toast region's
  role and politeness, focus staying on the invoker, the alert on failure,
  a dismissed toast staying hidden across a morph, the canonical route
  links, the reflected `?page=` without a history entry, the reload onto the
  same page and the local tab changing without a request. The first run
  failed on the protocol version and the toast display rule, as recorded
  above.
- NAV-006: `tests/load-more-continuity.test.ts` asserts the identity plan
  inserts only the new keyed rows, removes and moves none, and marks each
  entry as a live key; the browser case proves the kept node in every
  engine.

### Limits recorded

- Live pagination depends on protocol 2; an island that paginates through
  `url_intent` declares `minimum_protocol_version = 2`, and the framework
  refuses the reflection otherwise. The manual and the changelog say so.
- Toasts and load-more rows are loop keyed, so the mounting island exposes
  `filters::live_key`; a library component that renders inside a loop now
  carries that requirement in its header comment.
- The notification bell, the live feed and the account menu stay with the
  live-native commitment, as the commitment records.

## Mechanism demonstrations

Receipts on the built tree: FDB-001, FDB-002, FDB-006, NAV-001, NAV-002,
NAV-004 and NAV-006 (`20260915T041857Z`), the inherited UI and OVL tool
receipts (`20260915T041918Z`), FDB-003 with UI-006, UI-015 and FORM-004
(`20260915T041940Z`), the live:check receipts (`20260915T042142Z`), UI-017
(`20260915T042154Z`), and FDB-004 with NAV-003, UI-008, UI-012 and
LIVE-010 to LIVE-012 from the Live gate (`20260915T044033Z`).
