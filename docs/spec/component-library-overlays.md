# Component library - overlay and disclosure

Status: Draft
Prefix: OVL

Refines Live spec 23
(`crates/suprnova-live/docs/specs/suprnova-live/23-overlay-and-disclosure-components.md`):
disclosure and accordion, menus, popovers and tooltips, modal dialogs,
drawers and sheets, layering. Inherits every Agreed block of
`component-library-foundations.md`.

Components in complexity order. Presentational: tooltip (CSS, with a
dismissal enhancement), collapsible and accordion (`details`, `details
name` for single-open), popover (native Popover API), dropdown menu
(single level). Behavioral: dialog and confirm, sheet, drawer. The
tooltip's dismissal is this family's only custom element (OVL-008,
revised 2026-09-17); the anchor-positioning and popover-open-continuity
spike in the qualification host opens this family's commitment.

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

[OVL-002] The tooltip MUST show its bubble without script: a relative
wrapper with an absolute bubble on `:hover` and `:focus-visible`,
associated through `aria-describedby`. A custom-element enhancement MAY
add dismissal to that bubble.
Falsifier: the tooltip needs script to show its bubble, the bubble is not
referenced by `aria-describedby`, or the bubble stays hidden in a document
the enhancement never reaches.
Mechanism: `.cairn/mechanisms/ui-overlays`;
`.cairn/mechanisms/ui-live-check`.
Revised 2026-09-17: the first text read "The tooltip MUST be CSS only: a
relative wrapper with an absolute bubble on `:hover` and
`:focus-visible`, associated through `aria-describedby`", and its
falsifier refused script outright. That left a pointer user no way to
dismiss a bubble that covers content, which WCAG 2.2 success criterion
1.4.13 requires, so OVL-007's hoverable half shipped while the
dismissible half waited in `.cairn/next-iteration/`. The developer chose
the script enhancement in conversation on 2026-09-17 at 17:51 EDT,
answering "1" to the three ways the item recorded, over waiting for a
native hint popover the qualified engines do not all support and over a
placement that never obscures content.
Status: Agreed 2026-09-17

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

[OVL-007] The tooltip MUST keep its bubble visible while the pointer
moves from the trigger onto the bubble.
Falsifier: the bubble hides once the pointer moves from the trigger onto
it.
Evidence: `crates/suprnova-live/components/tooltip/tooltip.css` sets
`pointer-events: none` on the bubble.
Mechanism: `.cairn/mechanisms/live-library-review-remediation`.
Refines: OVL-002; Live spec 23, tooltips; WCAG 2.2 success criterion
1.4.13.
Reading: dismissing the bubble without moving the pointer or focus needs
script, which OVL-002 excludes; that change waits in next-iteration.
Status: Agreed 2026-09-17 by promotion the-review-of-live-and-the-component-library-is-remediated-in-one-commitment

[OVL-008] The tooltip MUST hide its bubble when the user presses Escape
while the pointer rests on the trigger or the bubble, or while focus is
inside the trigger, with neither the pointer nor focus moved. The tooltip
MUST show that bubble again the next time the user hovers or focuses its
trigger.
Falsifier: Escape leaves the bubble visible while the pointer rests on the
trigger, or a dismissed bubble stays hidden when the pointer leaves the
trigger and hovers it again.
Mechanism: `.cairn/mechanisms/ui-tooltip-dismissal`.
Refines: OVL-002; OVL-007; Live spec 23, tooltips are dismissible; WCAG
2.2 success criterion 1.4.13.
Reading: the enhancement OVL-002 now admits carries the family's only
custom element, and UI-020 binds it like every other.
Status: Agreed 2026-09-17
