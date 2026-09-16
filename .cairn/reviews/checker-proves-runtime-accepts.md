# Review - checker-proves-runtime-accepts

commitment: checker-proves-runtime-accepts
commit: fc6946b5
examined:
  - LIVE-025 against the built tree: the branch renderer's empty caller and the fail-closed check in `check_component`; the regressions `checker_proof_covers_the_view_after_an_empty_call_block` and `checker_proof_checks_caller_content`, the first failing against the reverted empty caller (`~/workspace2/scratchpads/live-key/mutation-empty-caller.log`); `live:check` over the dogfood app catching an undeclared action placed in the form gallery, which it had proved before.
  - LIVE-026: `BindingTiming::debounce`, the model macro, the compile-fail cases `invalid_debounce` and `unlisted_model_debounce`, and `debounce_is_limited_to_the_grammar_durations`; the dogfood galleries declaring 250 ms.
  - LIVE-027: `validate_error_target` and `live_error_target_is_a_declared_field_or_action`, beside the runtime's `scopeFor`.
  - The whole Live crate suite (940 tests), the framework's Live tests (113), the dogfood app suite (131), clippy on the touched crates, and the dogfood browser suite on chromium, firefox and webkit.
  - The session lock release kept on escalation `loop-035-sess-001`: `HeldSessionLock` and `an_abandoned_request_releases_the_session_lock`, which waits exactly 10 s against a mutant that never releases on drop (`mutation-session-drop.log`).
  - Live specs 03, 11 and 19, the manual sentence in seven locales, the changelog in seven locales and the translation lock.
findings:
  - resolved: The fixed checker exposed 24 errors in the form gallery, all under three names the component never declared correctly: the validation summary's `live:error` on the action `save` (LIVE-027 now accepts it), the file input and its field error bound to an undeclared upload field `avatar` (the gallery declares it with a PNG policy finalized by `save`, protocol 2, and its upload abilities), and the search field's template debounce against the field's 300 ms (both 250 ms, LIVE-026).
  - resolved: The backlog record said the checker accepts any debounce; the checker's grammar is closed like the runtime's, and the wider vocabulary was the model macro and `BindingTiming`. The 300 ms template looked accepted only because the empty call block hid the view. The decision record states the correction.
  - resolved: The first Live gate over the build failed on the form gallery on chromium: the forms page's stylesheets answered 503 after 10 s. Session blocking (SESS-001) held the session lock past a request the server abandoned, here the dashboard's event stream cancelled by the navigation, because the release after the write never ran on a dropped future. The held lock now releases on drop (`0e51a13e`), kept inside this commitment on escalation `loop-035-sess-001`.
  - resolved: The same run failed the form gallery's save case on firefox. The save form bound ten model controls and a Live request carries at most eight proposals, so the runtime's request validator refused the submit before any fetch and reported it as a network failure; the submit had never run in any browser. The case added in live-key-vocabulary counted a late model response as the submit and passed on timing. The form now keeps five model controls with the preferences outside it, and the case waits for the save request itself, failing against the old form (`mutation-save.log`) and passing nine of nine runs on three browsers (`e88a9596`). The cap itself, which leaves an application form with more than eight model controls without a working submit, is in the backlog.

## Build review, 2026-09-16

### Attacked: contradictions

- "Proved only when every element was checked" against limits: a limit that
  empties the branches reports itself, and the empty-branch check adds an
  error either way, so no limit path proves a component.
- "Fails to compile for any other duration" against existing applications:
  a field declaring an unlisted debounce could never be bound by a template
  the checker accepts, so the compile error replaces a view that could not
  pass `live:check`; the changelog names the narrowing.
- "`live:error` targets what the runtime resolves" against field rules: a
  name that is a declared field keeps `validate_field`, so a secret or
  server-only field is still refused; only an undeclared name falls through
  to the action check.

### Falsifiers, each demonstrated

- LIVE-025: an undeclared action after an empty call block is reported, and
  the same regression proves clean without it.
- LIVE-026: 300 ms fails `BindingTiming::debounce` and the macro.
- LIVE-027: a validation summary on `save` proves; `nowhere` and a secret
  field fail.

### Limits recorded

- The island target the runtime also resolves for error feedback is not
  accepted by the checker; no shipped component uses it.
- The eight-proposal request cap is unchanged and in the backlog.

## Mechanism demonstrations

Receipts on `fc6946b5`, from the stale check after escalation `loop-035-sess-001` (process 3415384, log `~/workspace2/scratchpads/live-key/check-soundness-3.log`), 38 requirements passing:

- LIVE-025, LIVE-026, LIVE-027, `checker-soundness`: `.cairn/evidence/runs/20260916T181350655Z-3415384`, the checker and timing tests and the macro compile-fail suite, exit 0.
- UI-009 and the `live:check` group, `ui-live-check`: `.cairn/evidence/runs/20260916T183107199Z-3415384`, every dogfood component proved with the fixed checker.
- The Live gate group (LIVE-010, LIVE-011, LIVE-012, UI-008, UI-012, OVL-005, FDB-004, NAV-003): `.cairn/evidence/runs/20260916T181724572Z-3415384`, pass on chromium, firefox and webkit; the failing run was `20260916T161220094Z-2630644`.
- The dogfood group, `ui-dogfood-tests`: `.cairn/evidence/runs/20260916T181715424Z-3415384`, pass.
- Every other stale requirement: pass, receipts from process 3415384 committed at `fc6946b5`; SESS-001 was not stale after its fix was committed and checked with the session-blocking tests.

