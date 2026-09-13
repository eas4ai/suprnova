# Review - component-library-foundations

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

### For the developer

The spec's "Rulings the developer still owes" list is written so that each
item names what a ruling authorizes. If any item is unclear, ask for it to
be explained another way before ruling.
