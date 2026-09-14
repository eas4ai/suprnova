# Review - component-library-foundations

## Specification review, 2026-09-14 (before building)

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

## Agreement record

UI-001 to UI-019 and FORM-001 to FORM-004 were confirmed as one set on
2026-09-13 at 11:08. The developer resumed the commitment on 2026-09-14
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
