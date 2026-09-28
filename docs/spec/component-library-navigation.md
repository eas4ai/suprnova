# Component library - navigation

Status: Draft
Prefix: NAV

Refines Live spec 22
(`crates/suprnova-live/docs/specs/suprnova-live/22-navigation-components.md`):
links and navigation actions, primary and responsive navigation,
breadcrumbs, tabs, pagination. Inherits every Agreed block of
`component-library-foundations.md`.

Components in complexity order. Presentational: header bar, footer,
sidebar with collapsible groups, breadcrumbs, tabs, pagination.
Behavioral: load more, account menu. No custom-element tier in this
family.

Out of the built-in set: the stepper (a complete block) and the command
palette (specialty), recorded in Live spec 22's revision of 2026-09-13;
the capabilities remain specified there for the separate project.

## Presentational tier

[NAV-001] Every navigation component MUST render internal destinations as
anchors with real route URLs. A navigation component MUST NOT render an
action as a link for style.
Falsifier: a shipped navigation item performs a Live action from an
anchor, or navigates from a button.
Mechanism: `.cairn/mechanisms/ui-live-check` and a grep over shipped
views for `live:click` on anchors.
Status: Agreed 2026-09-14

[NAV-002] The tabs component MUST require an explicit mode: local panels
with tablist semantics and local signals, or route tabs as anchors with
current-page semantics.
Falsifier: a tabs instance renders without a declared mode, or a local
tab makes a server request on change.
Mechanism: `.cairn/mechanisms/ui-live-check`; a browserless harness case
asserting no request on a local tab change.
Status: Agreed 2026-09-14

[NAV-003] The pagination component MUST render canonical page URLs in
route mode. In Live mode, the pagination component MUST reflect only the
current same-route query with `history.replaceState` and no per-page
history entry.
Falsifier: a Live-mode page change creates a history entry, or a
route-mode page link lacks a canonical URL.
Mechanism: one Playwright case per mode.
Status: Agreed 2026-09-14

[NAV-004] The sidebar and header bar MUST take current-route state from
server authority. The sidebar and header bar MUST hold only collapse
state locally.
Falsifier: a shipped navigation component computes "current" in the
browser.
Mechanism: a grep over shipped views; the dogfood document test asserts
`aria-current` in the canonical document.
Status: Agreed 2026-09-14

## Behavioral tier

[NAV-005] The account menu MUST render as a stitch slot under RenderCache.
The account menu MUST document that classification.
Falsifier: the account menu's markup appears in a shared cached shell.
Mechanism: the dogfood stitched-dashboard test extended with the account
menu.
Status: Agreed 2026-09-15

[NAV-006] The load-more control MUST be a button bound to a registered
action that appends to a keyed list. Every item already present MUST keep
its identity across the append. The control MUST expose its loading state
through `live:loading`. The control MUST leave the view when the server
reports no further page.
Falsifier: a shipped load-more control is an anchor, an existing item is
replaced by the append morph, or the control stays active after the last
page.
Mechanism: the navigation tool over the shipped views; a morph fixture
under `crates/suprnova-live/browser/tests/`.
Status: Agreed 2026-09-14

[NAV-007] A local tabs instance MUST select only its own tabs and panels,
whether another tabs instance is nested inside it or around it.
Falsifier: clicking a tab of local tabs nested in a panel of another local
tabs instance hides that outer panel.
Evidence: `crates/suprnova-live/components/tabs/tabs.js` selects every
descendant tab and handles the events that bubble from a nested instance.
Mechanism: `.cairn/mechanisms/live-library-review-remediation`.
Refines: NAV-002.
Status: Agreed 2026-09-17 by promotion the-review-of-live-and-the-component-library-is-remediated-in-one-commitment
