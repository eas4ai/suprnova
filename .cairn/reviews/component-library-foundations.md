# Review - component-library-foundations

commitment: component-library-foundations
commit: 275ad5ad5908fbe1c5b33e25fbc4e7445ef3e7b2
examined:
  - UI-001 to UI-019 and FORM-001 to FORM-004 against the closing tree, their mechanisms and the receipts recorded on it.
  - The 79 retained paths of escalation loop-035 (the v2.0.2 release history), against the library's declared inputs.
  - The five library decisions under docs/decisions/ and Live iteration 007.
findings: []

## Retention review, 2026-09-14 (after escalation loop-035)

The developer answered `ok` on escalation `loop-035` (committed as
`6e2af36c`), keeping the 79 paths that changed on main between the
commitment's activation (`2b526ada`) and the resumed base (`8a4b0c82`):
the workspace version bumps and READMEs of v2.0.2, CHANGELOG and its six
mirrors, the translation lock, the rustls lock update in the testing-off
probe fixture, the fanout module touched by the Astra remediation, and the
mirror typography fixes. Examined against the library:

- None of the retained paths is a library artifact. The stylesheet,
  components, checker expansion, `live:add`, the reserved namespace and
  the dogfood gallery all landed after `6e2af36c` (`781b63c1` to
  `44e7bed7`), so no library work rode in under the approval.
- Three retained paths are now declared inputs of `ui-tokens` for UI-007
  (`CHANGELOG.md`, `manual/cli.md`, `.manual-translations.lock`) because
  the library's own documentation edits landed in them later
  (`34193bc6`). Their receipts (`.cairn/evidence/UI-007/20260914T222502259Z`)
  were recorded after the approval and after those edits.
- The approval widened nothing: every mechanism's inputs are the ones
  declared for the library, and every receipt cited below postdates
  `6e2af36c`. The fresh receipts on the closing tree are UI-001 to UI-005
  and UI-007 (`20260914T222502Z`), UI-006, UI-015 and FORM-004
  (`20260914T222520Z`), UI-009, UI-014, UI-016 and FORM-001 to FORM-003
  (`20260914T222703Z`), and UI-008 and UI-012 with LIVE-010 to LIVE-012
  (`20260914T222344Z`); UI-010, UI-011, UI-013, UI-017, UI-018 and UI-019
  keep their receipts from the build, whose inputs have not changed since.

Nothing in the retained history contradicts an agreed requirement, and no
finding is open.

Identifier note: the foundations spec was renumbered at 10:55 on
2026-09-13 when the developer's rulings added four requirements. The
review below uses the identifiers as they were when it was written; the
map is UI-004 -> UI-008, UI-005 -> UI-009, UI-006 -> UI-010, UI-007 ->
UI-011, UI-008 -> UI-012, UI-009 -> UI-013, UI-010 -> UI-014, UI-011 ->
UI-015, UI-012 -> UI-016, UI-013 -> merged into UI-002 and UI-004, UI-014
-> UI-018, UI-015 -> UI-019. New: UI-004 (no literal visual values),
UI-005 (no style attributes), UI-006 (headless with the base layer
stripped), UI-007 (Tailwind preset), UI-017 (component directory and
manifest).

## Specification review, 2026-09-13 (before agreement)

Reviewed `docs/spec/live.md` (LIVE-001 to LIVE-015, Observed),
`docs/spec/component-library-foundations.md` (UI-001 to UI-015, Draft),
and `docs/spec/component-library-forms.md` (FORM-001 to FORM-008, Draft)
as the specification phase requires: contradictions between
requirements, falsifiers that would not catch their requirement's
violation, and requirements no mechanism can check. The family files for
navigation, overlays, feedback, and data display were reviewed for form
only; their content review happens when their commitment is drafted. The
spec lint ran clean after twenty-eight form findings were fixed across the
two rounds (two obligations in one sentence; an actor hidden inside a code
span).

### Attacked: contradictions

- UI-004 (library assets served as runtime feature artifacts) against the
  open distribution ruling (vendored presentational macros). Reading
  recorded: UI-004 binds the artifact Suprnova ships; a copy an
  application vendors is the application's own asset and outside UI-004.
  The requirement text stands; the ruling decides whether a vendored copy
  exists at all.
- UI-006 and UI-007 (light-DOM custom elements) against Live spec 20
  lines 110-111 (component JavaScript through local primitives or Stimulus
  controllers). A real contradiction, not a reading; listed as ruling 3 in
  the foundations spec and blocking UI-006, UI-007, and UI-014.
- FORM-001 (native controls) against Live spec 21's combobox and date
  capabilities: no contradiction, those controls are the family's
  custom-element tier (FORM-006 to FORM-008) and belong to a later
  commitment.
- UI-010 to UI-015 (explicit registration, reserved namespaces) against
  LIVE-005 and LIVE-006: consistent; they restate the engine's existing
  registry model and add reservations the engine does not enforce today
  (the `suprnova.` prefix rejection in UI-011 is new behavior).

### Attacked: falsifiers that would not catch a violation

- UI-002: "renders coherently" is not observable by a rule-presence check.
  The falsifier catches only a missing rule for a listed element; the
  coherence claim is a review judgment. Limit recorded; the developer may
  prefer a screenshot comparison in the qualification host, which would be
  its own mechanism.
- UI-008: the first draft's falsifier passed whenever the matrix ran at
  all. Tightened to require a Playwright case per component that uses a
  platform feature beyond plain HTML and CSS.
- UI-009: a grep for per-item mounts is a weak observer of "one island per
  widget". Recorded as weak; the dogfood stitched-dashboard test is the
  real evidence and the mechanism should name it once the datatable
  commitment exists.

### Attacked: requirements no mechanism can check

- LIVE-014 (a dated revision precedes a behavior change) is a process rule
  no script observes; Live's `check-specs.mjs` checks structure, not
  revisions. Observed, not Agreed; it stays a review item.
- LIVE-015 records an absence at `31fb0ead`. It has no ongoing mechanism
  and will be superseded by this commitment; it exists so the next agent
  knows the library did not exist before.

### Mechanism commands, verified or not

- `spec-lint`: ran clean on the draft.
- `live-contracts`: the three commands are what the repository gate's
  live-contracts step runs; not re-run here.
- `live-gate`: the Live gate script exists with 22 phases; not run here.
- `ui-light-dom`: a grep; trivially runnable; not run here.
- `ui-live-check`: the command shape comes from `suprnova-cli/src/main.rs:181-192`
  (`--templates`, `--allow-unproved`, `--timeout-secs`) and
  `suprnova-cli/src/commands/live_check.rs:83` (requires a project root,
  so it runs from `app/`). Not executed yet; the commitment's first check
  will show whether the dogfood helper builds under it.
- `ui-tokens`: built 2026-09-13 as `.cairn/tools/ui-tokens.mjs`,
  per-requirement results. Demonstrated (SPEC-022) on disposable
  fixtures: with no stylesheet at the declared path all three fail with
  "no stylesheet at ..."; a fixture with the shadow role deleted and the
  busy state keyed to a class fails UI-001 ("no custom property for role
  shadow") and UI-003 ("state selected by class: .is-busy; no selector on
  aria-busy") while UI-002 passes; the corrected fixture passes all
  three. The first run overwrote one failure reason with the next; fixed
  to accumulate reasons before recording this. Limit: the base-element
  and state-class checks are regular expressions over the stylesheet
  text, not a CSS parse; a selector split across lines or nested inside
  an at-rule with unusual spacing could evade them.
- `ui-tokens`, extended 2026-09-13 10:58 for the headless ruling
  (UI-002 layer membership, UI-004 no literal visual values, UI-005 no
  style attributes, UI-007 Tailwind preset). Demonstrated (SPEC-022) on a
  second fixture set: the violating stylesheet (a `border-radius: 6px`,
  a `#06c` color, a stray rule after the layer), a view with a `style`
  attribute, and a preset missing one token failed UI-002 ("a rule sits
  outside @layer suprnova-ui"), UI-004 ("border-radius literal; color
  literal; border-color literal"), UI-005, and UI-007 ("preset does not
  map --sn-color-accent"); the corrected set passed all six. Two false
  positives found and fixed before recording: the radius guard could be
  backtracked past (now declarations are parsed and each value judged),
  and a `:root` block stripped from inside `@media` left an empty media
  block that read as a stray rule. Limit: `@layer` detection assumes the
  layer block closes with a `}` at line start, which is how the shipped
  stylesheet will be formatted; a minified stylesheet would need the
  check to parse braces properly.

### Agreement

The developer walked the four open rulings one by one (2026-09-13,
10:20-10:54: styling c, distribution c, custom elements b', chart
charts-rs only, plus the five family items) and confirmed UI-001 to
UI-019 and FORM-001 to FORM-004 with their falsifiers as one set at
11:08. The six queued decisions were reviewed and their queue entries
removed in the same commit as the Agreed markers.

## Build review, 2026-09-14

Reviewed UI-001 to UI-019 and FORM-001 to FORM-004 against the tree the
commitment inherited on 2026-09-14 (release v2.0.2, main `8a4b0c82`) and the
four rulings recorded under `docs/decisions/` on 2026-09-13. Nothing of the
library existed in code: no stylesheet, no components directory, no
`live:add`, no reserved-namespace rule, and a checker that marked every
macro call unproved (`crates/suprnova-live/src/checker/branch.rs`,
`Node::Call | Node::Macro => DynamicStructureUnproved`), which would have
made UI-009 unreachable for macro-built views.

### Attacked: contradictions

- UI-008 asks for a row in "the reviewed artifact-size baseline"; the
  operations document records that artifact size is reported, not
  budgeted, and the docs contract enforces that sentence. Reading taken:
  the reviewed artifact table in
  `crates/suprnova-live/docs/implementation/iteration-004-operations.md` is
  the row, and the build reports the base's exact bytes like every other
  artifact. Recorded in Live spec 20's entry of 2026-09-14.
- UI-017 says a component is one directory under the reserved template
  root; static files are served from `public/`. Resolved by serving a
  vendored component's stylesheet and script from that directory at
  `/suprnova-ui/<component>/<file>` (`framework/src/live/ui_assets.rs`),
  so the directory stays the single copy.
- UI-015 says "a registry unit test in the library crate"; a component's
  origin is its compile-time crate name, so the framework's own tests
  register under the prefix successfully and the refusal can only be
  shown from a foreign crate. The dogfood application is that crate
  (`app/tests/live_dogfood.rs`); the acceptance sits in
  `framework/tests/live/library_namespace.rs`.
- FORM-002 names the checker as its mechanism for accessible names; the
  checker proves directive grammar and ownership, not label association.
  The gallery test asserts the association in the rendered markup
  (`the_gallery_keeps_names_and_state_without_the_base_layer`), and
  `live:check` proves the controls' directives; the checker rule for
  names is not written. Recorded as a limit below.

### Attacked: falsifiers that would not catch a violation

- UI-004's checker parses declarations and judges each value, so a
  `var(` guard cannot be backtracked past; comma-separated transition
  lists were rewritten as longhands because the checker refuses them,
  which is the stricter reading.
- UI-011 has no value-carrying enhancement in this commitment (the
  password reveal wraps a native control). The mechanism holds a static
  rule over every shipped element file so the requirement has a check
  before the first such enhancement (FORM-006 to FORM-008, later
  commitments).
- UI-006 is proved on the server render: names and state attributes are
  markup facts, so their presence without the stylesheet is the whole
  claim; the browser cases add the upgraded element and the layered
  stylesheet on each engine.

### Attacked: requirements no mechanism can check

- None left: every requirement of the commitment is named by exactly one
  mechanism (LOOP-056 forced UI-018 out of `ui-light-dom` into
  `ui-elements`).

## Agreement record, resumption

The developer resumed the commitment on 2026-09-14
at 16:01 ("let's move on to the component commitments") and answered
`ok` on escalation `loop-035` at 16:09, keeping the release history the
commitment inherited.

## Mechanism demonstrations

Safe violating examples, each shown against the mechanism before the
matching code existed or with a disposable probe:

- `ui-tokens`: the first receipts of 2026-09-14 (`20260914T200351895Z`)
  fail on a missing stylesheet; a probe stylesheet with `color: #fff` and
  `.active` under `crates/suprnova-live/components/` failed UI-003 and
  UI-004 before it was removed.
- `ui-live-check`: `.cairn/evidence/UI-009/20260914T211434479Z` (fail):
  five `dynamic_structure_unproved` diagnostics on the gallery's macro
  calls plus `invalid_modifier` on the dogfood avatar uploader, whose
  checker contract version was 1. The checker's negative fixtures
  (`macro-unknown-model.html`, `macro-missing.html`) refuse an unknown
  model passed through a macro and keep an unresolvable call unproved.
- `ui-elements`: a probe file defining `bad-thing` with a shadow root and
  `setFormValue` without `formAssociated` failed UI-011 and UI-018.
- `ui-dogfood-tests`: `the_registry_refuses_the_reserved_namespace_from_the_application`
  registers `suprnova.probe` from the app crate and asserts
  `RegistryErrorKind::ReservedName`.
- `ui-live-add`: the CLI test edits an installed view and asserts the
  second run keeps it; a third-party manifest claiming `suprnova-ui/` is
  refused.
- `live-gate`: the first two runs on the delivery commit failed
  (`.cairn/evidence/UI-008/20260914T204302426Z`, an engine test pinned to
  eight roles; `20260914T205734792Z`, a Playwright case that assumed every
  artifact is JavaScript); both were corrected and the third run passed.

Latest passing receipts:

- UI-001: `.cairn/evidence/UI-001/20260914T214447617Z` (pass) on `44e7bed7`
- UI-002: `.cairn/evidence/UI-002/20260914T214447617Z` (pass) on `44e7bed7`
- UI-003: `.cairn/evidence/UI-003/20260914T214447617Z` (pass) on `44e7bed7`
- UI-004: `.cairn/evidence/UI-004/20260914T214447617Z` (pass) on `44e7bed7`
- UI-005: `.cairn/evidence/UI-005/20260914T214447617Z` (pass) on `44e7bed7`
- UI-006: `.cairn/evidence/UI-006/20260914T214450076Z` (pass) on `44e7bed7`
- UI-007: `.cairn/evidence/UI-007/20260914T214447617Z` (pass) on `44e7bed7`
- UI-008: `.cairn/evidence/UI-008/20260914T211003156Z` (pass) on `cdc93067`
- UI-009: `.cairn/evidence/UI-009/20260914T211937378Z` (pass) on `d988ad95`
- UI-010: `.cairn/evidence/UI-010/20260914T214450767Z` (pass) on `44e7bed7`
- UI-011: `.cairn/evidence/UI-011/20260914T214450297Z` (pass) on `44e7bed7`
- UI-012: `.cairn/evidence/UI-012/20260914T211003156Z` (pass) on `cdc93067`
- UI-013: `.cairn/evidence/UI-013/20260914T214450521Z` (pass) on `44e7bed7`
- UI-014: `.cairn/evidence/UI-014/20260914T211937378Z` (pass) on `d988ad95`
- UI-015: `.cairn/evidence/UI-015/20260914T214450076Z` (pass) on `44e7bed7`
- UI-016: `.cairn/evidence/UI-016/20260914T211937378Z` (pass) on `d988ad95`
- UI-017: `.cairn/evidence/UI-017/20260914T214447364Z` (pass) on `44e7bed7`
- UI-018: `.cairn/evidence/UI-018/20260914T214450297Z` (pass) on `44e7bed7`
- UI-019: `.cairn/evidence/UI-019/20260914T214448481Z` (pass) on `44e7bed7`
- FORM-001: `.cairn/evidence/FORM-001/20260914T211937378Z` (pass) on `d988ad95`
- FORM-002: `.cairn/evidence/FORM-002/20260914T211937378Z` (pass) on `d988ad95`
- FORM-003: `.cairn/evidence/FORM-003/20260914T211937379Z` (pass) on `d988ad95`
- FORM-004: `.cairn/evidence/FORM-004/20260914T214450076Z` (pass) on `44e7bed7`

## Limits recorded

- **Accessible-name checker rule.** FORM-002's accessible names are
  asserted on the rendered gallery, not by a checker rule; a checker rule
  for label association is future work for the checker, not this
  commitment.
- **Value-carrying enhancements.** None ship here; UI-011's static rule
  waits for the first (input OTP, date picker, combobox in the form
  family's later tier).
- **Element helper.** UI-008 names "the element helper" among the shared
  bases; no shared helper exists yet because the one custom element ships
  self-contained. It becomes an artifact when the first widget needs it.
- **Origin crate is a compile-time name.** The reserved-namespace rule
  trusts `CARGO_PKG_NAME`; a crate cannot be named `suprnova` beside the
  dependency, which is the guarantee relied on.
- **Vendored asset caching.** `/suprnova-ui/<component>/<file>` answers
  with `must-revalidate` and a strong ETag rather than an immutable
  identity, because the application edits those files.
