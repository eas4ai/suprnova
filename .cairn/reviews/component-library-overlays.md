# Review - component-library-overlays

commitment: component-library-overlays
commit: 0ed49968e0170c6f45dc717fb681608a7a2839a4
examined:
  - OVL-001 to OVL-006 against the overlay tree, their mechanisms and the receipts recorded on it.
  - The spike's capability record on chromium, firefox and webkit at the pinned Playwright version.
  - The inherited foundations mechanisms rerun over the eight new component directories.
  - The checker change that decides literal-argument conditions, against the branch-limit regression.
findings:
  - resolved: The first browser run closed the spike's popover before the morph because the morph button sat outside it and a click outside a popover is its native light dismiss; the morph is now invoked from inside the popover. No component changed.
  - resolved: The dialog, sheet and drawer hosts used display contents and could not take focus, so the fallback focus after an invoker left the document never landed; the hosts are now rendered blocks. Fixed before the components were committed.
  - resolved: The overlay gallery crossed the checker's 256 branch states because every conditional in a macro counted as a branch even when the call's literal argument decided it; the checker now renders only the decided arm (spec 19, 2026-09-14).

## Spike, 2026-09-14

Recorded by `crates/suprnova-live/browser/e2e/overlay-spike.spec.ts` on the
three qualified engines at Playwright 1.62.1. Every engine reported the same
capability set: the `popover` attribute, `dialog` with `showModal`,
`closedBy`, `details name`, CSS anchor positioning (`anchor-name` and
`position-area`), and `commandForElement`. The popover and menu are
therefore positioned by the browser through the invoker's implicit anchor,
inside an `@supports (position-area: block-end)` block with a centred
fallback, and no positioning script ships. The same case proved an open
popover and an open keyed disclosure surviving a morph invoked from inside
the popover, an unkeyed disclosure following the server's closed state, and a
dialog returning focus to its invoker. The capability record describes the
Playwright engines, not the product floors: the documented floors (Chrome and
Edge 114, Firefox 128, Safari 17) hold `popover` and `dialog`, which is what
OVL-001 needs; anchor positioning below the current stable channels falls to
the centred fallback.

## Build review, 2026-09-14

### Attacked: contradictions

- OVL-001 against OVL-005: a native `dialog` has no declarative opener at the
  floor, so a script must call `showModal`. The primitive still owns the open
  state, containment and Escape; the vendored element only invokes it,
  closes on a close button or on a backdrop click where the browser lacks
  `closedBy`, and returns focus. The tool fails a script that writes the
  `open` or `hidden` attribute, toggles a class, handles Tab, or manages
  `inert`.
- OVL-004 against OVL-003: the menu's action item invokes a registered action
  and hides the menu with `popovertargetaction="hide"`; the open and close
  controls (invokers, summaries, close buttons) carry no directive, and the
  tool allows a directive only on an action item.
- OVL-006 against the runtime: templates write `live:key` for the checker
  while the runtime reads `data-suprnova-live-key`, and nothing maps them.
  The overlay roots write both. The gap is filed in the backlog and is not
  this commitment's to close.

### Falsifiers, each demonstrated

- OVL-001 to OVL-004: `.cairn/tools/ui-overlays.mjs` fails on a copy whose
  dialog script sets the `open` attribute, whose tooltip loses its
  `:focus-visible` rule, whose menu nests a list, and whose summary carries
  `live:click` (all four run on 2026-09-14 in a scratch copy; the tool also
  fails every requirement when the components or the gallery are absent,
  which the baseline receipt records).
- OVL-005: `e2e/app-dogfood.spec.ts` closes each modal with Escape and asserts
  focus on the trigger, then removes the sheet's trigger and asserts focus on
  the host; the first run failed on the unfocusable host, as recorded above.
- OVL-006: `tests/overlay-continuity.test.ts` asserts the preserved keyed root
  keeps its attribute, the unkeyed and the unpreserved roots do not, and a
  replaced region forces replacement; the spike and the gallery case prove it
  in the browser.

### Limits recorded

- The accordion's single-open mode degrades to independent disclosures below
  Chrome 120, Firefox 130 and Safari 17.2 (spec 23 revision).
- The three modal hosts ship near-identical scripts because a vendored
  component is self-contained; a shared helper is the element helper the
  roadmap leaves to the live-native commitment.
- The menu carries no menu role and no arrow-key handling on purpose: it is
  a list of links and buttons, which spec 23 allows for simple lists.

## Mechanism demonstrations

Receipts on the overlay tree: OVL-001 to OVL-004 (`20260915T005218Z`),
OVL-006 (`20260915T005219Z`), the inherited UI and FORM receipts of the same
run, and OVL-005 with UI-008, UI-012 and LIVE-010 to LIVE-012 from the Live
gate (`20260915T011106Z`).
