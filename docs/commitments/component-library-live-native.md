# Component library live native

Slug: component-library-live-native
Requirements: FORM-005, FORM-006, FORM-007, FORM-008, FDB-005, NAV-005
Rests on: UI-001, UI-002, UI-003, UI-006, UI-008, UI-009, UI-010, UI-011, UI-012, UI-013, UI-014, UI-017, UI-018, UI-019, LIVE-008, LIVE-009, LIVE-011, FORM-001, FORM-002, FORM-003, FORM-004, FDB-001, NAV-003

## Goal

Suprnova ships the live-native family of its Live component library, the
components that only make sense on the running runtime: the upload widget
over the shipped upload protocol, the live feed and the notification bell
over the async-updates transport with an honest degraded state, the
account menu as a documented stitch slot under RenderCache, and the
custom-element tier of the forms family, input OTP, date picker and
combobox, as light-DOM, form-associated `sn-` elements that enhance
native controls the page works without.

Drafted 2026-09-15 after `cairn wake` reported
component-library-data-display Done and the roadmap named this item
next; the developer ruled on 2026-09-14 at 23:35 that every remaining
commitment is agreed, and the six requirements are recorded as one set
through escalation `form-005`. The roadmap line also names the
server-rendered chart; it landed under DATA-004 in item 6 and is not
rebuilt here.

Not in this commitment: a JavaScript helper library as the element base.
The glossary's Draft entry names an "Elena-class helper"; the tree holds
no such library, Live spec 20 keeps component JavaScript on Live
primitives, and the password input already ships a plain light-DOM
element, so the three enhancements are plain `HTMLElement` subclasses
with `ElementInternals`, defined only by their own vendored file. Also
out: tag and token input, multi-value combobox, the complex blocks the
roadmap excludes.

## Deliverables

- Six component directories under `crates/suprnova-live/components/`
  (upload, live-feed, notification-bell, account-menu, input-otp,
  date-picker, combobox), each with a manifest, an Askama macro view, CSS
  under the `suprnova-ui` layer with `--sn-` tokens only, and, for the
  three enhancements, one JavaScript file that defines its `sn-` tag and
  nothing else.
- The upload widget renders the upload domain's states through the
  shipped `__live/upload` protocol and the runtime's upload manager; it
  adds no transfer path and claims completion only after the finalizing
  action.
- The live feed and notification bell project the transport's status
  (live, reconnecting, degraded, retired) into visible text and an
  `aria-live` region; a retired stream never presents itself as live.
- The account menu island documents its stitch-slot classification and
  joins the dogfood stitched dashboard as its fourth slot.
- The input OTP is one native input with per-character presentation; the
  date picker's strips are native radio groups in scroll-snap containers;
  the combobox is the accessible combobox pattern over a native input and
  list, and a stale response never replaces a newer query's options.
- The dogfood gallery gains a live-native page mounting every shipped
  component, with a Playwright case per engine that blocks each element's
  script and proves the native control still submits.
- Manual: the component library section of `manual/live.md` lists the
  family and the enhancement rule; the six mirrors are re-stamped under
  the translation lock; the 2.0.2 changelog section gains the entry.
- Live specs 08, 14 and 21 gain dated "Decisions and revisions" entries
  for any ruling the build needs; iteration 011 records the scope.

## Records

- Requirements: `docs/spec/component-library-forms.md` (FORM-005 to
  FORM-008), `docs/spec/component-library-feedback.md` (FDB-005),
  `docs/spec/component-library-navigation.md` (NAV-005), on the Agreed
  foundations, forms, overlays, feedback, navigation and data-display
  records.
- Mechanisms, declared after the agreement, one owner per requirement
  (LOOP-056): a live-native tool under `.cairn/tools/` reporting per
  requirement over the shipped views and their scripts (FORM-006,
  FORM-007), a vitest fixture over the combobox query sequencing
  (FORM-008), a vitest fixture over the feed's status projection
  (FDB-005), and `ui-dogfood-tests` for the upload widget's end-to-end
  case and the stitched dashboard (FORM-005, NAV-005); the inherited
  `ui-live-check`, `live-gate`, `ui-light-dom`, `ui-elements`,
  `ui-islands`, `ui-tokens`, `ui-live-add` and `ui-framework-tests` runs
  cover the new directories.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/component-library-live-native.md`.

## Tests that prove it

- The upload widget's dogfood test drives create, chunk, complete and the
  finalizing action through `handle_request` and reads every state name
  from the rendered view; no request leaves the `__live/upload` route
  (FORM-005).
- The tool fails a scratch copy whose OTP splits into several inputs,
  whose strip is not a radio group in a scroll-snap container, or whose
  element script defines a second tag or attaches a shadow root
  (FORM-006, FORM-007).
- The combobox fixture resolves an older query after a newer one and
  asserts the newer options stand; the shipped view carries the listbox
  roles (FORM-008).
- The feed fixture projects `retired`, `reconnecting` and `degraded`
  into distinct visible states, none of them "live" (FDB-005).
- The stitched dashboard test counts four slots and finds the account
  menu absent from the shared shell (NAV-005).

## Done when

Every mechanism above passes on a committed tree, the review is recorded,
and `cairn wake` says Done.
