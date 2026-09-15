# Component library - overlay and disclosure

Status: Draft
Prefix: OVL

Refines Live spec 23
(`crates/suprnova-live/docs/specs/suprnova-live/23-overlay-and-disclosure-components.md`):
disclosure and accordion, menus, popovers and tooltips, modal dialogs,
drawers and sheets, layering. Inherits every Agreed block of
`component-library-foundations.md`.

Components in complexity order. Presentational: tooltip (CSS), collapsible
and accordion (`details`, `details name` for single-open), popover
(native Popover API), dropdown menu (single level). Behavioral: dialog
and confirm, sheet, drawer. No custom-element tier in this family; the
anchor-positioning and popover-open-continuity spike in the qualification
host opens this family's commitment.

Out of the built-in set: nested submenus, recorded in Live spec 23's
revision of 2026-09-13; the submenu behavior remains specified there for
a separate project.

## Presentational tier

[OVL-001] Every disclosure, popover, and dialog component MUST use the
native primitive (`details`, the `popover` attribute, `dialog`) for its
open state, focus containment, and light dismiss before any script.
Falsifier: a shipped overlay reimplements open state or focus containment
in script where the native primitive provides it.
Mechanism: a grep over shipped views for the native elements; one
browserless harness case per component with the helper disabled.
Status: Agreed 2026-09-14

[OVL-002] The tooltip MUST be CSS only: a relative wrapper with an
absolute bubble on `:hover` and `:focus-visible`, associated through
`aria-describedby`.
Falsifier: the tooltip requires script to appear, or the bubble is not
referenced by `aria-describedby`.
Mechanism: `.cairn/mechanisms/ui-live-check`; a grep over the tooltip
view.
Status: Agreed 2026-09-14

[OVL-003] The dropdown menu MUST be single level, with real anchors for
navigation items and buttons or registered actions for action items.
Falsifier: a shipped menu nests a submenu, or an action item is an anchor.
Mechanism: `.cairn/mechanisms/ui-live-check` and a grep over shipped
views.
Status: Agreed 2026-09-14

## Behavioral tier

[OVL-004] Overlay open state MUST stay browser-local. A server action
invoked from an overlay MUST own only its authoritative effect.
Falsifier: opening or closing a shipped overlay makes a Live request with
no declared server effect.
Mechanism: a browserless harness case asserting no request on open and
close.
Status: Agreed 2026-09-14

[OVL-005] A dialog, sheet, or drawer MUST return focus to its invoker on
close, or to a safe fallback when the invoker disappeared.
Falsifier: after close, focus rests on `body` while the invoker is still
in the document.
Mechanism: one Playwright case per component per qualified engine.
Status: Agreed 2026-09-14

[OVL-006] Open state MUST survive a compatible morph only under a stable
`live:key` scope.
Falsifier: a keyed open overlay closes on a morph that did not touch it,
or an unkeyed one persists across a replaced region.
Mechanism: the morph continuity fixtures under
`crates/suprnova-live/browser/tests/` extended with overlay cases.

Status: Agreed 2026-09-14