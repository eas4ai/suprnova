# Component library feedback and navigation

Slug: component-library-feedback-and-navigation
Requirements: FDB-001, FDB-002, FDB-003, FDB-004, FDB-006, NAV-001, NAV-002, NAV-003, NAV-004, NAV-006
Rests on: UI-001, UI-002, UI-003, UI-006, UI-009, UI-010, UI-012, UI-013, UI-014, UI-017, OVL-001, LIVE-008, LIVE-009, LIVE-011

## Goal

Suprnova ships the feedback family and the navigation family of its Live
component library on the foundations and the overlays: alert, skeleton,
spinner, progress, empty state, toast and flash region; header bar,
footer, sidebar with collapsible groups, breadcrumbs, tabs, pagination and
load more. Every feedback surface represents a real runtime or server
state (a `live:loading` target, a lazy island, session flash, a
server-rendered reason) and announces proportionately; every navigation
surface keeps native route semantics (anchors with real URLs, `aria-current`
from the server, no client router) and holds only collapse and local tab
state in the browser.

Drafted 2026-09-14 after the developer named the commitment at 23:02
("component-library-feedback-and-navigation is the next commitment"); the
ten requirements await his agreement as one set through escalation
`fdb-001`. Two are new in this draft because the roadmap names components
no requirement covered: FDB-006 (progress) and NAV-006 (load more). Tabs
was listed under the overlays item of the roadmap but never in OVL-001 to
OVL-006 and was not built there; NAV-002 specifies it and it lands here.

Not in this commitment: the notification bell and live feed (FDB-005) and
the account menu (NAV-005), which the roadmap places in
`component-library-live-native`.

## Deliverables

- Thirteen component directories under `crates/suprnova-live/components/`
  (alert, skeleton, spinner, progress, empty-state, toast, flash-region,
  header-bar, footer, sidebar, breadcrumbs, tabs, pagination, load-more;
  the sidebar's collapsible groups reuse the `details` primitive of
  OVL-001), each with a manifest, an Askama macro view, CSS under the
  `suprnova-ui` layer with `--sn-` tokens only, and JavaScript only where
  a native primitive leaves a gap (local tabs' arrow-key and selection
  behavior, the toast queue's pause and dismissal).
- The flash region reads the framework session's flash bag so a redirect
  after an accepted action presents its outcome once and never on a
  restored document; the toast region is a `role="status"` live region
  that announces once and never receives focus.
- Route pagination renders canonical page URLs; Live pagination rides
  the runtime's reflected URL intent (`history.replaceState`, no history
  entry) rather than a component script.
- The dogfood gallery gains a feedback page and a navigation page
  mounting every shipped component, with one empty-state reason per
  document test and both pagination modes.
- Manual: the component library section of `manual/live.md` lists both
  families; the six mirrors are re-stamped under the translation lock;
  the 2.0.2 changelog section gains the entry.
- Live specs 22 and 24 gain a dated "Decisions and revisions" entry for
  any ruling the build needs.

## Records

- Requirements: `docs/spec/component-library-feedback.md` (FDB) and
  `docs/spec/component-library-navigation.md` (NAV), on the Agreed
  foundations in `docs/spec/component-library-foundations.md` and the
  Agreed overlays in `docs/spec/component-library-overlays.md`.
- Mechanisms, declared after the agreement: a feedback tool and a
  navigation tool under `.cairn/tools/` reporting per requirement over the
  shipped views and the galleries (FDB-001, FDB-002, FDB-004, FDB-006,
  NAV-001, NAV-002, NAV-004, NAV-006), `ui-dogfood-tests` (FDB-003),
  `live-gate` (NAV-003 through the Playwright matrix), a morph fixture
  mechanism (NAV-006), and the inherited `ui-live-check`, `ui-light-dom`,
  `ui-elements`, `ui-islands` and `ui-tokens` runs over the new
  directories.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-feedback-and-navigation.md`.

## Tests that prove it

- Every alert variant carries a role chosen by urgency and a non-color
  cue (an icon or a prefix in text) that differs between variants
  (FDB-001).
- Every spinner and skeleton in a shipped view is bound to a
  `live:loading` target or sits inside a lazy island; the harness shows
  a bound spinner only after the runtime's delay (FDB-002).
- A dogfood document test per empty reason (empty, no results, no
  permission, disconnected) finds the reason text, and the no-permission
  document offers no create action (FDB-003).
- The harness fires a toast, finds one announcement in the status region
  and focus unchanged; a critical error in the gallery also renders in
  the persistent alert (FDB-004).
- The progress view is a native `progress` element with a label; the
  indeterminate instance carries no `value` (FDB-006).
- No shipped navigation view carries `live:click` on an anchor or an
  `href` on a button (NAV-001).
- Every tabs instance declares `mode="local"` or `mode="route"`; the
  harness changes a local tab and records zero Live requests (NAV-002).
- Playwright on chromium, firefox and webkit: route-mode page links are
  canonical URLs; a Live-mode page change leaves `history.length`
  unchanged and the query reflected (NAV-003).
- No shipped navigation view computes the current item in script; the
  dogfood document carries `aria-current` from the server (NAV-004).
- The morph fixture appends a page to a keyed list and keeps every
  existing node; the control is absent after the last page (NAV-006).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
