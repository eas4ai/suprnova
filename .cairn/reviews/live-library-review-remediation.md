# Review - live-library-review-remediation

commitment: live-library-review-remediation
commit: 9f48d020
examined:
  - FORM-008, FORM-011, FORM-012: `combobox.js` (conditional writes, the remote mode keyed on `data-sn-query`, the observer on `hidden` only), the macro's `remote` parameter, the gallery's `#[rendering]` filter; the component cases in `e2e/components/combobox.spec.ts` on chromium, firefox and webkit, four of five failing on the previous script.
  - FORM-009, FORM-010, LIVE-032: the ten value macros and their app copies, `readBindingGroup` and `readCheckboxGroup` with the `data-suprnova-live-collection` marker, the form gallery's authority and `#[rendered]` release; `form_009_the_form_gallery_renders_the_island_values_into_its_controls` (fails with the number input's value removed), `tests/model-form-groups.test.ts` (fails on the previous runtime), and `e2e/app-dogfood-forms.spec.ts`, whose group and reset cases fail on a build without the keys or the authority marker and pass 27 of 27 across three repeats.
  - LIVE-031: the generated `bind_models` issues, `with_binding_issues` bounded by the engine's configured ceiling, the action, sync-only and instanced paths in `framework/tests/live/dogfood_forms.rs`, and the engine unit case for the bound.
  - LIVE-033 to LIVE-036: `in_key_alphabet` against the runtime's `SAFE_KEY` literal, `live_key_digest`, the checker's id rule per island, the unchecked-state conditionals, per-arm shadowing and the 33-byte digest marker; `checker_regressions.rs` cases that fail on the committed checker before each change.
  - FDB-007, NAV-007, OVL-007, UI-020, UI-024: the toast, tabs and tooltip changes, the AbortController pattern with `connectedMoveCallback` in every library element, and the select indicator with its forced-colors fallback; the component cases, 21 of them failing on the previous scripts and the forced-colors case failing without its rule.
  - DATA-006: the 1e9 value bound and the sweep in `view_charts.rs`.
  - UI-021 to UI-023: `try_live_ui_assets_from`'s directory check and `try_live_ui_assets` under a base path without templates; `live:add`'s install record, written after each file, its strict read, the bare manifest path, and the one-handle read of third-party files; every `ui_02x_` case failing on the previous sources.
  - An independent read-only review of 5fdcef19 to 754f15f9 (3 defects, 3 risks, 6 nits), each finding verified against the code before it was acted on.
  - The Live crate and macro suites (1065), the app suite (132), the framework Live tests (128), the CLI suite (686), the browser unit suite (897), ESLint, tsc, Prettier, the reproducible build, `live:check` on the dogfood app (11 proved), workspace clippy (no findings), the manual chapter and changelog in seven locales and the translation lock.
findings:
  - resolved: The first `cairn check` found `live:check` at the branch limit on both galleries: every value control's `{% if %}` doubled the branch states, so any real form built on FORM-009's macros would fail the check. The checker now expands a conditional that renders only `checked`, `selected` or the correction marker once, because no check reads them; anything else still branches, and a regression pins both sides.
  - resolved: The draft dogfood cases for FORM-010 and the reset passed without the keys or the authority marker, because every gallery control sends its update as the user edits. The cases now hold a request on its way to open the window a selection is unanswered in, and use an edit the island refuses for the reset; both fail on a build without the fix.
  - resolved (review D1): a checkbox group rendered with one option proposed a boolean, which LIVE-031 then refused, so an untouched submit could never run. The group marks its boxes and the runtime reads a marked box as a list member.
  - resolved (review D2): `live:add --manifest manifest.json` failed on the empty parent path.
  - resolved (review D3): the textarea macro lost a leading line feed to the HTML parser.
  - resolved (review R1): the symlink check and the read named the file twice; the read now goes through one handle that must be the checked file on Unix. The race itself is not reproducible in a test; the symlink cases cover the check.
  - resolved (review R2): the gradient indicator vanished under forced colors; the select returns to its native indicator there.
  - resolved (review R3, N1, N2, N4, N5, N6): the instanced action path is covered, the bag bound follows the engine, shadowing is per arm, the digest key is measured at 33 bytes, the install record follows each write, and `try_live_ui_assets` itself is tested.
  - resolved (review N3, in part): ids are judged per island and not inside `template`; a literal id inside a loop stays refused, because the checker cannot tell that an element in a loop renders once, and the manual says so.
  - resolved: The Live gate's correctness-delay scanner refused the new component case files: a dynamic tag name in the dialog, sheet and drawer fixtures, and the `setTimeout` deadline that bounds a step a frozen renderer never finishes. The fixtures are literal, and the deadline carries the scanner's watchdog exception; Playwright's own action timeouts cannot interrupt a frozen renderer (the FORM-011 case hung past five minutes without it).
  - recorded: `.cairn/backlog/the-browser-runtime-drops-an-edit-equal-to-its-last-proposal-after-a-server-render-changed-the-control.md`, found while proving the reset.

## Build review, 2026-09-17

### Attacked: contradictions

- "Render the island's current value" against continuity: continuity keeps
  what the user typed over a render's value, so a render that must replace it
  says so with the correction marker, and only the render answering the reset
  carries it (`form_009_` asserts the render after it does not).
- "A group proposes a list" against LIVE-031: without the marker, a group of
  one option proposed a boolean the list field refuses; the marker removes the
  dependence on the option count.
- "The checker proves what the runtime accepts" against the unchecked-state
  expansion: the skipped attributes are ones no check reads, so no diagnostic
  can depend on them; an arm with any other content still branches.

### Falsifiers, each demonstrated

- FORM-011: a query with no match leaves the page responsive in three engines.
- FORM-012: a listbox answering "sao" with a name lacking the substring shows it.
- FORM-009: an input mounted at 1 renders `value="1"`, and an untouched save
  proposes 1 and 50; without the value parameter the case fails.
- FORM-010: a selection made while a search update is on its way survives that
  update's re-render; without the keys it is cleared.
- LIVE-031: null for `u64` and a boolean for a list are field errors and save
  does not run, on seed, sync and instance paths.
- LIVE-032: two checked topic boxes propose `["releases", "security"]`.
- LIVE-033 to LIVE-036, DATA-006, UI-020 to UI-024, FDB-007, NAV-007, OVL-007:
  each regression fails on the code before its change.

### Limits recorded

- The Live gate's `upload_file_provider` retirement case failed once in the
  first check and fails about one whole-file run in twenty on this machine; the
  upload code is untouched by this commitment.
- The Live gate's Firefox activity feed case failed once under the full
  matrix (744 passed) and passed five of five alone; the async transport is
  untouched by this commitment.
- A literal id inside a loop is refused even where the loop renders it once.

## Mechanism demonstrations

Receipts on `9f48d020`, from check process 2795057, every mechanism passing:

- FORM-008 to FORM-012, FDB-007, NAV-007, OVL-007, DATA-006, UI-020 to UI-024, LIVE-031 to LIVE-036, `live-library-review-remediation`: `.cairn/evidence/runs/20260917T155100992Z-2795057`, the component cases on chromium, firefox and webkit, the dogfood form cases, the runtime unit cases, `live:check` on the dogfood application, the manual text, and one labeled nextest run per requirement and target.
- The Live gate group: `.cairn/evidence/runs/20260917T155304693Z-2795057`, pass.
- `ui-live-check`: `.cairn/evidence/runs/20260917T160628744Z-2795057`, every dogfood view proved.
- Every other stale requirement: pass, receipts from process 2795057 committed at `92e2703b`.
