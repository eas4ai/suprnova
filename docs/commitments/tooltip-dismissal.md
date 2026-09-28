# The tooltip can be dismissed where it is shown

Slug: tooltip-dismissal
Requirements: OVL-002, OVL-008
Specified from: `.cairn/next-iteration/the-tooltip-cannot-be-dismissed-without-moving-the-pointer-or-focus-while-ovl-002-keeps-it-css-only.md`
Rests on: OVL-007, UI-010, UI-012, UI-017, UI-020

## Goal

A tooltip bubble that covers content can be dismissed where it is shown,
with neither the pointer nor focus moved, as WCAG 2.2 success criterion
1.4.13 requires, and the tooltip still shows that bubble with no script of
its own in the document. Specified 2026-09-17 in the phase between loops,
on the developer's choice of the script enhancement over a native hint
popover the qualified engines do not all support and over a placement that
never obscures content.

## Deliverables

- `crates/suprnova-live/components/tooltip/tooltip.js` defines one light-DOM
  custom element that hides its wrapper's bubble on Escape while the pointer
  rests on the trigger or the bubble, or while focus is inside the trigger,
  and shows it again on the next hover or focus of that trigger. It binds
  through one `AbortController` per connection with an empty
  `connectedMoveCallback`, as UI-020 requires, and holds no form value.
- `tooltip.css` hides a dismissed bubble through an attribute the element
  writes, so a document without the script renders the tooltip exactly as
  before; `tooltip.html` carries the element around the wrapper, and
  `manifest.json` names the script and the element.
- The vendored copies under `app/templates/suprnova-ui/tooltip/` match, and
  the overlay gallery keeps `live:check` clean.
- `e2e/components/tooltip.spec.ts` passes on chromium, firefox and webkit:
  the script-absent case for OVL-002 and the two Escape cases for OVL-008,
  which fail on the tree this commitment starts from.
- `manual/live.md` states the dismissal in the overlay family, mirrored in
  six locales; the 2.0.2 changelog records it in seven locales, with the
  translation lock re-stamped.

## Review before agreement

Attacked before the developer agreed:

- OVL-008 against OVL-007: OVL-007 keeps the bubble visible while the
  pointer moves onto it, and OVL-008 hides it on an explicit keypress.
  They meet only when the pointer rests on the bubble and Escape is
  pressed, where dismissal is the user's own request.
- The revised OVL-002 against OVL-001: OVL-001 binds the disclosure,
  popover and dialog components to their native primitives before script.
  The tooltip has no native primitive for dismissal in the qualified
  engines, which is why the item reached this phase.
- The revised OVL-002 against UI-010 and UI-011: the enhancement is light
  DOM and carries no value, so UI-011's form association does not reach it.
- The falsifiers against their mechanism: a Playwright case parks the
  pointer, presses Escape and reads the bubble's visibility, and a second
  case does the same from keyboard focus; both observe the violation
  directly. The script-absent half of OVL-002's falsifier needed a harness
  that loads a component's stylesheet without its script, which the
  component harness now takes as an option.
- Found: OVL-008's first draft said only "the tooltip MUST be dismissible",
  which no mechanism could observe without naming the key, the place the
  pointer rests, and what happens next; the agreed text names all three.

## Records

- Requirements: `docs/spec/component-library-overlays.md` (OVL-002 revised,
  OVL-008).
- Mechanism: `.cairn/mechanisms/ui-tooltip-dismissal`; `ui-overlays` keeps
  OVL-002's static half.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/tooltip-dismissal.md`.

## Done when

The mechanism passes on a committed tree, the Live gate and the repository
gate pass, the review is recorded, and `cairn wake` says Done.
