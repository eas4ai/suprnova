# Manual check - comparing the manual with the code

Status: Draft
Prefix: MAN

The manual is checked against the code, not the other way round. The
feature map (`feature-map/surface.jsonl`) is generated from the source at a
recorded commit; `feature-map/tools/manual_check.py` resolves every code
reference in the English manual against it; `feature-map/tools/manual_triage.py`
joins each unresolved reference with a verdict a person or agent checked by
hand against the source. `feature-map/README.md` documents the commands, the
outcomes and the verdict classes. This file owns the contract: what the
comparison must guarantee, and how a finding is remediated.

Two rulings from the developer, in conversation on 2026-09-27, shape the
remediation half: "Nonexistent features need to be cut from the manual....
it is a manual", with the proviso that each cut is checked, as it is made,
for a feature that may be mistakenly unimplemented; and those gaps are filed
as GitHub issues about the code ("Not the docs the code").

Terms. A *reference* is one code span, or one name inside a fenced block,
that the check extracts. An *outcome* is what the check concluded about it.
A *verdict* is the hand-checked classification of an unresolved reference,
with source evidence. The *gap check* is the search, made before a cut, for
evidence that the missing thing was meant to exist or should. A *code gap*
is a finding whose fix belongs in the code.

## The comparison

[MAN-001] The source at the surface's recorded commit MUST be the
authority. A disagreement between the manual and the code MUST be settled
by reading the source; the manual, the CHANGELOG, a spec, or anyone's
memory of the API MUST NOT settle it.
Falsifier: A verdict's evidence cites no source location, or cites only the manual or the CHANGELOG for what the code does.
Status: Draft

[MAN-002] The check MUST run against a surface built from the current
source. When `feature-map/meta.json`'s `source_rev` is not the last commit
that touched the source paths, the check MUST refuse to run.
Falsifier: `manual_check.py` produces findings while `source_rev` lags the source.
Status: Draft

[MAN-003] Every English chapter under `manual/` MUST be read through a
CommonMark parser, and every Rust fenced block through tree-sitter-rust.
Every inline code span and every fenced block in `rust`, `bash`, `sh`,
`shell`, `console`, `env`, `dotenv`, `html`, `askama`, `jinja` and
`svelte` MUST yield its references. Fences in any other language are not
checked; that is a known hole, not coverage.
Falsifier: A code reference in a checked context produces no row in the check output.
Status: Draft

[MAN-004] Every extracted reference MUST receive exactly one outcome from
the set `manual_check.py`'s docstring defines, and that docstring MUST stay
the single definition of each outcome.
Falsifier: A row carries an outcome outside the set, or a second definition of an outcome disagrees with the docstring.
Status: Draft

[MAN-005] Every reference with outcome `missing`, `wrong_path` or `hidden`
MUST have a verdict in `manual-triage-verdicts.json`, with a class and
evidence that cites the source by file and line. The triage run MUST fail
while any such reference lacks one.
Falsifier: `manual_triage.py` exits 0 while a `missing`, `wrong_path` or `hidden` finding has no verdict.
Status: Draft

[MAN-006] A change that fixes or removes the manual text a verdict
describes MUST remove that verdict in the same commit. A committed triage
run MUST report no stale verdicts.
Falsifier: `manual_triage.py` reports a non-empty `stale_verdicts` at a committed state.
Status: Draft

[MAN-007] A problem the check cannot see - a wrong default, a real API
used wrongly, a wrong claim about behavior - MUST be recorded under
`extra` with its own locations, under the same evidence rule as MAN-005.
An `extra` entry does not go stale on its own; the change that fixes it
MUST remove it.
Falsifier: A hand-found manual error is neither fixed nor listed in `extra`.
Status: Draft

[MAN-008] The comparison covers the English chapters only. Translations,
`README.md`, rustdoc and the CHANGELOG are outside it, and no report MUST
claim otherwise.
Falsifier: A report or commit message presents a translation or the README as checked.
Status: Draft

## Remediation

The manual documents what a reader can do at the recorded commit. What the
code should do but does not is tracked as an issue against the code, never
as manual text.

The gap check (MAN-101) searches, in this order:

1. a public equivalent under another name or path (`fmap search`,
   `fmap show`); if one exists, the finding is a wrong name, not a gap;
2. the code's own docs and comments that point at the item;
3. the CHANGELOG, for a claim that it shipped;
4. `docs/spec`, `docs/commitments` and `docs/decisions`;
5. `manual/parity.md`'s status for it, and the Laravel map for a Laravel
   equivalent;
6. git history. The history begins at the import commit `9f48d02`
   (2026-09-17), so absence from it proves nothing about earlier work.

Each finding is remediated by this table (MAN-102):

| Finding | Manual | Code |
|---|---|---|
| Wrong name or path; the public equivalent exists | use the real name or path | nothing |
| Does not exist; no evidence it was meant to, and no gap worth filing | cut the passage | nothing; `gap: none` |
| Does not exist, and the gap check found intent (shipped, agreed, or pointed at by the code's docs) or a real gap (security, a Laravel equivalent that parity claims) | cut the passage, or rewrite it around what works today | issue; `gap: issue` |
| Exists but is broken (`code_bug`) | describe what works today, or cut | issue; `gap: issue` |
| Exists, public but `#[doc(hidden)]`, and a reader needs it (MAN-104) | leave it | issue to make it supported API |

When an issue is fixed, the change that fixes it restores the manual text.

[MAN-101] Before the manual text for a reference the source does not have
is changed, the gap check MUST run in the order listed above, and its
result MUST be recorded in the verdict: the evidence names what was
searched, and the `gap` field is `none` or `issue`.
Falsifier: An `error` or `code_bug` verdict has no `gap` field, or its evidence names no search.
Status: Draft

[MAN-102] The manual MUST NOT describe anything a reader cannot use at the
recorded commit, and each finding MUST be remediated by the table above.
Falsifier: A manual passage shows a call, flag, macro or setting that fails at the recorded commit.
Status: Draft

[MAN-103] An issue filed under MAN-102 MUST describe the code: its title
names the code's defect or missing capability, and its body gives what
happens or what is missing, the cause with file and line, where the
manual, the CHANGELOG or the code's docs claim it, and a suggested fix. It
MUST NOT ask for a manual change; the documentation pass makes that
change. Existing issues MUST be searched first, and the verdict MUST record
the issue number in `issue`.
Falsifier: An issue's requested change is to the manual, or a `gap: issue` verdict has no `issue` number.
Status: Draft

[MAN-104] A `hidden` verdict MUST say who the item serves.
Framework-internal - a test seam for the framework's own suite
(`*_for_test`, `_test_*`), macro plumbing (`__*`), internal dispatch, or a
reset the framework itself calls: the manual drops the name and may keep
describing the behavior. Reader-needed with no public alternative - an app
has to name it, as with `seed::clear` (eas4ai/suprnova#63): the manual
keeps it and an issue asks for it to become supported API. A public
equivalent exists: the manual switches to it and the verdict is `error`,
as with `#[suprnova::__async_trait]` for `#[suprnova::async_trait]`.
Falsifier: A `hidden` verdict does not say which of the three the item is.
Status: Draft

[MAN-105] A remediation change MUST do only what its findings need.
Manual fixes go in the documentation pass; code changes go through their
issue, in their own change, unless the developer rules otherwise. A
documentation pass MUST NOT add features, services or infrastructure.
Falsifier: A documentation-pass commit changes behavior under `framework/`, `crates/`, `suprnova-cli/` or `suprnova-macros/`.
Status: Draft

[MAN-106] Every change to the manual MUST re-run the check and the triage
in the same change and commit with no unreviewed findings and no stale
verdicts. A change that also touches source MUST regenerate the surface
and restamp `source_rev` first.
Falsifier: Re-running the check and triage on a committed state disagrees with the committed triage output.
Status: Draft

## Mechanisms

No block here names a mechanism yet; each gets one when it is agreed.
What exists and what is still to build:

- MAN-003, MAN-004: `feature-map/tools/manual_check.py`.
- MAN-005: `feature-map/tools/manual_triage.py` exits non-zero and lists
  the unreviewed references.
- MAN-002: to build. `fmap` warns through `freshness()`;
  `manual_check.py` has no guard.
- MAN-006: to build. `manual_triage.py` reports stale verdicts but exits 0.
- MAN-101, MAN-103: to build. `manual_triage.py` should require `gap` on
  `error` and `code_bug` verdicts, and `issue` when `gap` is `issue`.
- MAN-001, MAN-007, MAN-008, MAN-102, MAN-104, MAN-105, MAN-106: review.

## Open questions

- Several chapters carry framework-contributor material: the render cache
  test-seam tables in `render-cache-operations.md`, the "where the code
  lives" tables in `eloquent-serialization.md`. Under MAN-104 their hidden
  names go. Does that material belong in the manual at all, or in the
  crates' own docs?
- When an English passage is cut or rewritten, what happens to the six
  translations?
