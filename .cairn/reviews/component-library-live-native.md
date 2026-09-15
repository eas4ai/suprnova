# Review - component-library-live-native

commitment: component-library-live-native
commit: a82af6cb
examined:
  - FORM-005 to FORM-008, FDB-005 and NAV-005 against the built tree, their mechanisms and the receipts recorded on it.
  - The inherited foundations, forms, overlay, feedback, navigation, data-display, live:check, live:add, framework and dogfood mechanisms rerun over the seven new component directories, the live-native page and the dashboard's fourth slot.
  - The three element scripts for a shadow root, a second tag, a form value of their own, and for what they do after a morph.
  - The upload widget's end-to-end path through create, chunk, complete and the finalizing action, and the Playwright case on chromium, firefox and webkit with every element script blocked.
findings:
  - resolved: The gallery view crossed the checker's branch limit because six optional macro attributes multiplied over seven loops; every attribute those macros render is now a required parameter, and the single-file upload widget dropped an unused multiple flag.
  - resolved: The upload create request was refused (upload_authorization_denied) until the dogfood authorization defined the gallery's upload and stream abilities beside the avatar uploader's.
  - resolved: The stitched dashboard test compared two principals' documents after stripping only each island's opening tag; the account menu's body names the principal and carries the session's CSRF token, so the test now strips that slot whole, which is what NAV-005 says a stitch slot is.
  - resolved: The morph stripped the elements' upgrade marker and re-added the combobox input's datalist reference, so the popup reopened after a selection and the cells went blank after a re-render; the three wrappers are keyed with live:preserve.self, the scripts re-apply their changes from a MutationObserver writing only what differs, and the combobox keeps its popup closed after a selection until the user types again.
  - resolved: The strip radios are visually hidden, so the browser case clicks their labels; a strip selection is proved with the script blocked before the date input is filled, because the model round-trip re-renders the strips.
  - resolved: The Live gate's first run over the family failed 28 browser cases. The dogfood host's pooled sqlite::memory: connection was replaced when a browser closed a connection mid-request and came back without tables; the host now keeps a file in a temporary directory on one connection. The dashboard's cases expected three islands and the "no library component" case ran against a document that now mounts the account menu; they expect four and the public page.
  - resolved: An action's morph copied the server's disconnected default over the status the runtime had announced and stripped the root's stream state, so the feed and bell read disconnected over a current stream until the next tick (FDB-005). The morph now keeps the root state and the status text while a stream is projected; spec 14 records the rule, the fixture proves the preserve decisions, the browser case asserts the status after an action, and the rebuilt bundles carry reviewed integrity pins.
  - noted: Two races outside the commitment went to the backlog with their evidence: a model proposal queued behind an in-flight action is sent with the authority captured when it was queued (the combobox case now selects from its query's results, the FORM-008 flow), and concurrent requests on one session write back last-writer-wins (the flash case waits for the dashboard's islands).
  - noted: The three enhancements never call ElementInternals: every one wraps a native control that carries the value, so UI-011 does not apply to them and the escalation's "with ElementInternals" phrasing was stricter than the build needed; the spec 21 revision records the actual rule.

## Build review, 2026-09-15

### Attacked: contradictions

- FORM-005 against the runtime: the widget must show every state yet own no
  transfer. It renders every state name as text and the CSS selects the one
  the runtime's progress root names through data-live-upload-state; the
  dogfood test drives create, chunk, complete and the action over the
  reserved route and asserts the finalizer commits nothing until the action.
- FORM-006 against auto-advance: per-cell inputs would be the natural
  implementation and would split the code across controls. The code is one
  native input with a transient model; the cells are aria-hidden
  presentation and the script only reads the input.
- FORM-007 against a calendar widget: the strips are native radios in
  scroll-snap fieldsets with legends, so tap, click and arrow keys select
  with no script; the script composes the three parts into the date input
  and mirrors a typed date back.
- FORM-008 against stale answers: the listbox carries the query it was
  rendered for and the element refuses one that is not the input's current
  text; the fixture proves the decision and the sequence without a
  document, and the datalist keeps the control usable with the script
  blocked.
- FDB-005 against the runtime's ownership of status: the runtime already
  announces every stream state into the status element and writes the state
  on the island root, so the views render the disconnected message as the
  default and color the degraded, reconnecting and closed states; the
  fixture drives the runtime's feedback layer over the feed's markup and
  reads the announcements back.
- NAV-005 against the shared shell: the account menu is its own
  identity-bound island, the stitched dashboard stores four slots, and the
  shell comparison holds once the slot is stripped whole.

### Falsifiers, each demonstrated

- FORM-006, FORM-007: `.cairn/tools/ui-live-native.mjs` fails on scratch
  copies whose OTP renders a second input, whose OTP script defines a second
  tag or writes the input's value, whose strips lose scroll-snap, whose date
  picker renders a free input, or whose date script attaches a shadow root
  (six mutations run on 2026-09-15).
- FORM-008: `tests/combobox-stale-results.test.ts` rejects a result for an
  older or longer query and an answer with an older sequence.
- FDB-005: `tests/live-feed-status.test.ts` reads "Updates degraded",
  "Reconnecting to updates" and "Updates closed" from the runtime and fails
  a view whose default status says live or current.
- FORM-005, NAV-005: the dogfood tests assert the finalizer count before and
  after the action and four stored slots with an identical shell.

### Limits recorded

- The combobox ships single-value with local filtering over the
  server-rendered options; server-backed filtering goes through the input's
  model binding and the island's re-render, which the stale rule covers
  through data-sn-query.
- The date strips offer the years, months and days the island passes; the
  picker does not compute month lengths, and the native date input rejects
  an impossible date.
- The account menu's sign-out route is the dogfood application's own POST
  under its CSRF middleware; the library ships the form, not the route.

## Mechanism demonstrations

Every receipt below was recorded by one `cairn check` over the 55 requirements
on commit 824c3919 (the Live gate group included) and committed at a82af6cb,
after two earlier runs whose failures are the resolved findings above
(receipts 3161bb66 and b8e8696d). All 55 pass, on 2026-09-15 (UTC times):
DATA-001 (18:20:07Z), DATA-002 (18:20:07Z), DATA-004 (18:20:07Z), DATA-003 (18:20:07Z), UI-006 (18:20:30Z), UI-015 (18:20:30Z), FORM-004 (18:20:30Z), FDB-003 (18:20:30Z), DATA-005 (18:20:30Z), FORM-005 (18:20:30Z), NAV-005 (18:20:30Z), FDB-001 (18:20:30Z), FDB-002 (18:20:30Z), FDB-006 (18:20:30Z), LIVE-010 (18:40:40Z), LIVE-011 (18:40:40Z), LIVE-012 (18:40:40Z), UI-008 (18:40:40Z), UI-012 (18:40:40Z), OVL-005 (18:40:40Z), FDB-004 (18:40:40Z), NAV-003 (18:40:40Z), FDB-005 (18:40:41Z), LIVE-008 (18:42:01Z), LIVE-009 (18:42:01Z), UI-009 (18:42:01Z), UI-014 (18:42:01Z), UI-016 (18:42:01Z), FORM-001 (18:42:01Z), FORM-002 (18:42:01Z), FORM-003 (18:42:01Z), FORM-006 (18:42:02Z), FORM-007 (18:42:02Z), FORM-008 (18:42:02Z), NAV-001 (18:42:03Z), NAV-002 (18:42:03Z), NAV-004 (18:42:03Z), NAV-006 (18:42:03Z), OVL-001 (18:42:03Z), OVL-002 (18:42:03Z), OVL-003 (18:42:03Z), OVL-004 (18:42:03Z), OVL-006 (18:42:04Z), UI-001 (18:42:09Z), UI-002 (18:42:09Z), UI-003 (18:42:09Z), UI-004 (18:42:09Z), UI-005 (18:42:09Z), UI-007 (18:42:09Z), UI-010 (18:42:10Z), UI-011 (18:42:10Z), UI-018 (18:42:10Z), UI-013 (18:42:10Z), UI-017 (18:43:09Z), UI-019 (18:43:20Z).
