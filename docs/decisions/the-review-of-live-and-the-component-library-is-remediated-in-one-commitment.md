# The review of Live and the component library is remediated in one commitment

Level: Consequential
Decided by: Shawn
Promotes: the-combobox-freezes-the-page-when-the-typed-text-matches-no-option
Rests on: FORM-008 FORM-009 FORM-010 FORM-011 FORM-012 FDB-007 NAV-007 OVL-007 DATA-006 UI-020 UI-021 UI-022 UI-023 UI-024 LIVE-031 LIVE-032 LIVE-033 LIVE-034 LIVE-035 LIVE-036
Would be wrong if: a promoted finding is not a defect of the shipped code at c97da623, or its remedy needs an Agreed requirement's text changed, which belongs to a specification phase and not to this promotion

## Decision

Chosen 2026-09-17 by the developer, in conversation, after the adversarial review of Live and the component library that preceded the 2.1.0 release: "create a commitment for all of these items that require remediation", and "These are backlog items" (07:51 EDT). The review's eighteen confirmed findings were captured to the backlog (77368588, ad54131d); one commitment, live-library-review-remediation, promotes all of them. The item named in the Promotes line is the most severe; every other item carries Promoted to: live-library-review-remediation. Two review findings were refuted and are not promoted: a missing focus ring in forced-colors mode (Chromium's emulation shows one), and a let binding that shadows a macro parameter (Askama refuses to compile it).

Each item, the requirement it drafts, and that requirement's falsifier:

- the-combobox-freezes-the-page-when-the-typed-text-matches-no-option: FORM-011, the combobox stays responsive while its input holds text no option matches. Falsifier: typing a query that matches no option leaves the page unable to run a script within three seconds.
- the-form-family-renders-no-value-from-island-state-so-edit-forms-show-empty-controls-group-selections-vanish-on-re-render-and-the-checkbox-group-never-saves: FORM-009, the form-family macros render the island's current value into each control. Falsifier: the form gallery mounts a quantity of 1 and its number input renders empty, or a submit that changed no control proposes a value other than the one the island holds. FORM-010, a radio group and a checkbox group keep a selection the user has not yet sent across a re-render. Falsifier: a checked topic box is cleared by a model update of another field. LIVE-032, the runtime proposes the list of checked values for a field more than one checkbox binds. Falsifier: checking two topic boxes proposes a boolean.
- a-model-proposal-that-fails-to-decode-is-dropped-silently-and-the-action-reports-success: LIVE-031, an undecodable proposal is a validation error on its field and the action does not run. Falsifier: null for a u64 field or a boolean for a list field returns an accepted outcome with no validation entry, or the action runs.
- keys-starting-with-an-underscore-hyphen-dot-or-colon-pass-live-check-and-the-live-key-filter-then-disconnect-the-island-at-its-first-morph: LIVE-033, the checker, the live_key filter, and the runtime accept one key alphabet. Falsifier: a key the checker and filter accept, such as -1 or _draft, makes the runtime refuse the morph.
- the-checker-proves-a-macro-whose-loop-variable-shadows-one-of-its-parameters: LIVE-036, a name a loop or match arm binds inside a macro body is checked as that binding. Falsifier: a macro looping for name in names over a binding written with name, called with a literal, is proved.
- the-combobox-never-suppresses-a-listbox-rendered-for-an-older-query-and-its-stale-result-mechanism-tests-a-path-the-shipped-macro-never-takes: FORM-008 as Agreed; its falsifier, an older response's options render after a newer query's, gains a mechanism that drives the shipped element.
- a-toast-times-out-under-the-pointer-and-can-hide-while-its-dismiss-button-has-focus: FDB-007, the toast region holds a toast's timer while the pointer is over any part of it or focus is inside it. Falsifier: moving from the text to the padding lets the toast time out under the pointer, or it hides while its dismiss button has focus.
- library-custom-elements-rebind-their-listeners-when-a-morph-moves-them-which-disables-the-password-reveal: UI-020, a library custom element keeps one set of listeners and observers however often a morph moves it. Falsifier: after a move, one click on the password reveal toggles twice.
- vendored-component-stylesheets-and-scripts-are-read-from-the-working-directory-at-runtime-and-no-deployment-path-ships-them: UI-021, vendored component assets are served to an application started outside its project directory, or the application refuses to start. Falsifier: an application whose working directory holds no templates directory answers 404 for a vendored script and starts cleanly.
- one-data-derived-key-with-a-byte-outside-the-key-alphabet-fails-the-whole-island-render: LIVE-035, a shipped filter turns any value into a stable accepted key, and the manual states the live_key failure. Falsifier: no shipped filter keys a row by an email address, or the manual omits the failure.
- element-ids-inside-an-island-are-validated-by-the-runtime-and-never-by-the-checker: LIVE-034, the checker and the runtime agree on the ids an island may hold. Falsifier: an island holding the id _top or user[email] passes live:check and its first morph fails.
- render-chart-panics-inside-charts-rs-for-values-from-1e12: DATA-006, the chart renderer returns an error instead of panicking for finite values. Falsifier: render_chart with a value of 1e12 panics.
- clicking-an-inner-tab-of-nested-local-tabs-hides-the-outer-panel-that-holds-it: NAV-007, local tabs select only their own tabs and panels. Falsifier: clicking a nested tab hides the outer panel.
- the-select-chevron-is-invisible-in-the-dark-color-scheme: UI-024, the select indicator draws in the scheme's text color. Falsifier: in the dark scheme the indicator draws in black on the dark surface.
- the-combobox-hides-server-results-whose-text-does-not-contain-the-typed-query: FORM-012, the combobox shows every option of a listbox the server rendered for its input's text. Falsifier: a listbox rendered for "sao" holding a name without that substring shows no option.
- the-css-tooltip-cannot-be-hovered-or-dismissed: OVL-007, the bubble stays visible while the pointer moves onto it. Falsifier: the bubble hides once the pointer reaches it. Dismissing the bubble without moving the pointer or focus needs script, which OVL-002's Agreed text excludes, so that part waits in next-iteration for a specification phase and is not built under this promotion.
- live-add-reports-an-unedited-older-library-file-as-edited-locally-and-its-only-way-forward-also-overwrites-real-edits: UI-022, live:add replaces an installed file the application has not edited when the shipped file changes. Falsifier: an unedited older install keeps the older file or reports it edited.
- live-add-copies-the-target-of-a-symlink-in-a-third-party-component-into-the-project: UI-023, live:add refuses a third-party file that is a symbolic link or resolves outside its manifest's directory. Falsifier: a symlinked script installs the linked file's bytes.

Rejected: one commitment per item, which would stage 2.1.0 behind eighteen loops for defects the developer named together; and releasing first, which would ship a combobox that freezes the page.

## Realized by

- 5fc3d651  live: the combobox stays responsive for text no option matches, shows every option the server answered, and hides an answer to older text (FORM-011, FORM-012, FORM-008)
- e3a5a79c  live: every library custom element keeps one set of listeners and observers however a morph moves or reconnects it (UI-020)
- c6cfecdb  live: a toast holds while the pointer is anywhere on it or focus is inside, nested tabs act on their own tabs, the tooltip bubble takes the pointer, and the select indicator follows the text color (FDB-007, NAV-007, OVL-007, UI-024)
- 371f53df  live: the checker, the key filter, and the runtime share one key alphabet and one id rule, live_key_digest keys any value, a loop binding shadows a macro argument, and a chart refuses a value past 1e9 (LIVE-033 to LIVE-036, DATA-006)
- 452b7128  live: a model proposal its field cannot decode answers a validation error on that field, and the action it accompanies does not run (LIVE-031)
- f3bcf1bc  live: the form controls render the island's values, a group selection the server has not answered survives a re-render, and a checkbox group proposes the list of checked values (FORM-009, FORM-010, LIVE-032)
- 754f15f9  live: an application that cannot read its vendored components refuses to start, live:add replaces a file the application never edited, and a third-party file that is a symbolic link is refused (UI-021, UI-022, UI-023)
- ca45ee6a  cairn: the live-library-review-remediation mechanism runs each group of requirement-named cases once and reports every requirement from the cases that name it
- 48bfd4b4  docs: the Live chapter and the 2.0.2 changelog record the review remediation, in seven locales
- 993a4d8f  cairn: the review remediation mechanism declares framework/Cargo.toml, which names the live_dogfood_forms test target LIVE-031's case runs in
- 1003935f  live: live:check expands a control's rendered state once, binds a shadowing name only where it is bound, measures a digest key at its real length, and judges ids per island (FORM-009, LIVE-033, LIVE-034, LIVE-036)
- 2a44590e  live: a checkbox group of one option proposes a list, a textarea keeps a leading line feed, and the select keeps an indicator in forced colors (LIVE-032, FORM-009, UI-024)
- a59f5d53  live: refused proposals join the validation bag within the engine's configured ceiling, an instanced action with one is covered, and try_live_ui_assets is proved under a base path without templates (LIVE-031, UI-021)
- 7a9e6e58  cli: live:add reads a manifest named in the working directory, reads each third-party file through the handle it checked, and records every file as it writes it (UI-022, UI-023)
- 8f649633  docs: a checkbox group of any size proposes a list, and live:check refuses a literal id inside a loop, in seven locales
- 8b7d38f6  cairn: the review remediation mechanism runs live:check on the dogfood application and the checker and engine cases for FORM-009 and LIVE-031
- c7c12fdd  live: the component cases name the dialog, sheet and drawer fixtures literally and mark their step deadline as a failure-only watchdog, as the correctness-delay scanner requires
