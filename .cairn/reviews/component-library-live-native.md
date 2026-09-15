# Review - component-library-live-native

commitment: component-library-live-native
commit: e81fce64
examined:
  - FORM-005 to FORM-008, FDB-005 and NAV-005 against the built tree, their mechanisms and the receipts recorded on it.
  - The inherited foundations, forms, overlay, feedback, navigation, data-display, live:check, live:add, framework and dogfood mechanisms rerun over the seven new component directories, the live-native page and the dashboard's fourth slot.
  - The three element scripts for a shadow root, a second tag, a form value of their own, and for what they do after a morph.
  - The runtime's scheduler and transport under a proposal typed while its predecessor is in flight, the checker's resolution of feedback directive targets, and the morph's treatment of the runtime's stream status, each against the Live specs they refine.
  - The upload widget's end-to-end path through create, chunk, complete and the finalizing action, and the Playwright case on chromium, firefox and webkit with every element script blocked.
findings:
  - resolved: The gallery view crossed the checker's branch limit because six optional macro attributes multiplied over seven loops; every attribute those macros render is now a required parameter, and the single-file upload widget dropped an unused multiple flag.
  - resolved: The upload create request was refused (upload_authorization_denied) until the dogfood authorization defined the gallery's upload and stream abilities beside the avatar uploader's.
  - resolved: The stitched dashboard test compared two principals' documents after stripping only each island's opening tag; the account menu's body names the principal and carries the session's CSRF token, so the test now strips that slot whole, which is what NAV-005 says a stitch slot is.
  - resolved: The morph stripped the elements' upgrade marker and re-added the combobox input's datalist reference, so the popup reopened after a selection and the cells went blank after a re-render; the three wrappers are keyed with live:preserve.self, the scripts re-apply their changes from a MutationObserver writing only what differs, and the combobox keeps its popup closed after a selection until the user types again.
  - resolved: The strip radios are visually hidden, so the browser case clicks their labels; a strip selection is proved with the script blocked before the date input is filled, because the model round-trip re-renders the strips.
  - resolved: The Live gate's first run over the family failed 28 browser cases. The dogfood host's pooled sqlite::memory: connection was replaced when a browser closed a connection mid-request and came back without tables; the host now keeps a file in a temporary directory on one connection. The dashboard's cases expected three islands and the "no library component" case ran against a document that now mounts the account menu; they expect four and the public page.
  - resolved: An action's morph copied the server's disconnected default over the status the runtime had announced and stripped the root's stream state, so the feed and bell read disconnected over a current stream until the next tick (FDB-005). The morph now keeps the root state and the status text while a stream is projected; spec 14 records the rule, the fixture proves the preserve decisions, the browser case asserts the status after an action, and the rebuilt bundles carry reviewed integrity pins.
  - resolved: A model proposal queued behind an in-flight one superseded it, and the runtime discarded the superseded request's accepted response, so the island's revision and snapshot fell behind the server, the next proposal was sent against the consumed snapshot, the server answered 409 and the runtime reloaded the document; the queue also waited for the next stream tick because the transport only restarted it after an applied response. The runtime now applies a superseded response's authority without a render and restarts the queue after every settlement; spec 11 records the decision and the dogfood case types while a proposal is in flight and proves no reload and two accepted responses (developer's `ok` on escalation `form-008`).
  - resolved: The Live gate's correctness delay scan refused the fixed wait the in-flight proposal case first used; the gallery renders the country field's queued and loading feedback through live:queued.show and live:loading.show, and the case waits on the "Search pending" hint.
  - resolved: live:check refused those hints as unknown actions because the checker resolved every feedback target as an action; it now accepts one of the owner's model-bound fields, as spec 11 scopes targeted feedback, with a regression test for the proved and the unknown case and a spec 19 decision.
  - resolved: Concurrent requests on one session write back last-writer-wins, so a flash set by a redirect can be lost to a request that started earlier; on the developer's `ok` to escalation `form-008` this is routed outside the commitment as SESS-001 in `docs/spec/sessions.md`, Agreed 2026-09-15, with its own commitment `session-request-serialization` as roadmap item 8, and the flash case waits for the dashboard's islands to connect until that lands.

### Observed, no finding

The three enhancements never call ElementInternals: every one wraps a native control that carries the value, so UI-011 does not apply to them and the escalation's "with ElementInternals" phrasing was stricter than the build needed; the spec 21 revision records the actual rule.

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
on commit 2dc59f5a (the Live gate group included) and committed at e81fce64,
after four Live gate runs whose failures are the resolved findings above
(receipts 3161bb66, b8e8696d, 9a9e8898 and d5245d6b, all committed). All 55
pass, on 2026-09-15 (UTC times): DATA-001 (19:34:08Z), DATA-002 (19:34:08Z), DATA-004 (19:34:08Z), DATA-003 (19:34:09Z), UI-006 (19:36:25Z), UI-015 (19:36:25Z), FORM-004 (19:36:25Z), FDB-003 (19:36:25Z), DATA-005 (19:36:25Z), FORM-005 (19:36:25Z), NAV-005 (19:36:25Z), FDB-001 (19:36:25Z), FDB-002 (19:36:25Z), FDB-006 (19:36:25Z), LIVE-010 (19:54:34Z), LIVE-011 (19:54:34Z), LIVE-012 (19:54:34Z), UI-008 (19:54:34Z), UI-012 (19:54:34Z), OVL-005 (19:54:34Z), FDB-004 (19:54:34Z), NAV-003 (19:54:34Z), FDB-005 (19:54:35Z), LIVE-008 (19:54:49Z), LIVE-009 (19:54:49Z), UI-009 (19:54:49Z), UI-014 (19:54:49Z), UI-016 (19:54:49Z), FORM-001 (19:54:49Z), FORM-002 (19:54:49Z), FORM-003 (19:54:49Z), FORM-006 (19:54:50Z), FORM-007 (19:54:50Z), FORM-008 (19:54:50Z), NAV-001 (19:54:51Z), NAV-002 (19:54:51Z), NAV-004 (19:54:51Z), NAV-006 (19:54:51Z), OVL-001 (19:54:52Z), OVL-002 (19:54:52Z), OVL-003 (19:54:52Z), OVL-004 (19:54:52Z), OVL-006 (19:54:52Z), UI-001 (19:54:57Z), UI-002 (19:54:57Z), UI-003 (19:54:57Z), UI-004 (19:54:57Z), UI-005 (19:54:57Z), UI-007 (19:54:57Z), UI-010 (19:54:57Z), UI-011 (19:54:58Z), UI-018 (19:54:58Z), UI-013 (19:54:58Z), UI-017 (19:55:56Z), UI-019 (19:56:06Z).
