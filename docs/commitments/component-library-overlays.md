# Component library overlays

Slug: component-library-overlays
Requirements: OVL-001, OVL-002, OVL-003, OVL-004, OVL-005, OVL-006
Rests on: UI-001, UI-002, UI-003, UI-006, UI-009, UI-010, UI-012, UI-013, UI-017, LIVE-008, LIVE-009, LIVE-011

## Goal

Suprnova ships the overlay and disclosure family of its Live component
library on the foundations: tooltip, collapsible and accordion, popover,
single-level dropdown menu, dialog and confirm, sheet, and drawer. Every
open state is native (`details`, the `popover` attribute, `dialog`) before
any script, browser-local, focus-safe on close, and keyed for morph
continuity; every view is checker-clean, in light DOM, vendored by
`live:add`, and proven on the three qualified engines.

Drafted 2026-09-14 after the developer named the family as the next
commitment ("overlays?", 19:29); the six requirements await his agreement
as one set through escalation.

## Deliverables

- The spike first: a page in the Live test host that opens a popover and
  a dialog on chromium, firefox and webkit at the pinned Playwright
  version, records whether the `popover` attribute, `dialog`, `details
  name` and CSS anchor positioning are present, and whether an open
  popover survives a compatible morph; its result decides the positioning
  approach and is recorded in the review before any component is written.
- Seven component directories under `crates/suprnova-live/components/`
  (tooltip, collapsible, accordion, popover, dropdown-menu, dialog, sheet,
  drawer), each with a manifest, an Askama macro view, CSS under the
  `suprnova-ui` layer with `--sn-` tokens only, and JavaScript only where
  the native primitive leaves a gap (focus return, keyed continuity).
- The dogfood gallery gains an overlays page mounting every shipped
  overlay so `live:check` and the browser matrix exercise the real set.
- Manual: the component library section of `manual/live.md` lists the
  family; the six mirrors are re-stamped under the translation lock; the
  2.0.2 changelog section gains the entry.
- Live spec 23 gains a dated "Decisions and revisions" entry for any
  ruling the build needs.

## Records

- Requirements: `docs/spec/component-library-overlays.md` (OVL), on the
  Agreed foundations in `docs/spec/component-library-foundations.md`.
- Mechanisms: `.cairn/mechanisms/ui-overlays-native` (OVL-001, OVL-002,
  OVL-003: a grep and harness run over the shipped views),
  `ui-overlays-harness` (OVL-004: the browserless harness asserts no
  request on open and close), `live-gate` (OVL-005 through the Playwright
  matrix), `ui-overlays-morph` (OVL-006: the morph continuity unit
  fixtures), and the inherited `ui-live-check`, `ui-light-dom`,
  `ui-islands`, `ui-tokens` runs over the new directories.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-overlays.md`.

## Tests that prove it

- A grep over the shipped overlay views finds `details`, `popover` or
  `dialog` on every open-state root and no script-owned open state; a
  browserless harness case per component opens and closes it with the
  helper disabled (OVL-001).
- The tooltip view carries no script, its bubble is referenced by
  `aria-describedby`, and the harness shows it on focus (OVL-002).
- The dropdown menu view has no nested menu and every action item is a
  button or registered action, every navigation item an anchor (OVL-003).
- The harness records zero Live requests across open and close of every
  overlay (OVL-004).
- A Playwright case per dialog, sheet and drawer on chromium, firefox and
  webkit closes it and finds focus on the invoker, then removes the
  invoker before closing and finds focus on the declared fallback
  (OVL-005).
- Morph fixtures keep a keyed open overlay across an untouched morph and
  close an unkeyed one inside a replaced region (OVL-006).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
