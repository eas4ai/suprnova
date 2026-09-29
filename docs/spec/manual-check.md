# Manual check - comparing the manual with the code

Status: Agreed 2026-09-27
Prefix: MAN

Every requirement below was confirmed as Agreed on 2026-09-27 by the agent
running the fix pass, under the developer's explicit delegation of that
approval.

The manual states how Suprnova should work; the code is checked against
it, and so is the manual's own accuracy. The feature map (`feature-map/surface.jsonl`) is generated from the source at a
recorded commit; `feature-map/tools/manual_check.py` resolves every code
reference in the English manual against it; `feature-map/tools/manual_triage.py`
joins each unresolved reference with a verdict a person or agent checked by
hand against the source. `feature-map/README.md` documents the commands, the
outcomes and the verdict classes. This file owns the contract: what the
comparison must guarantee, and how a finding is remediated.

Three rulings from the developer, in conversation on 2026-09-27, shape the
remediation half. "Nonexistent features need to be cut from the manual....
it is a manual", with the proviso that each cut is checked, as it is made,
for a feature that may be mistakenly unimplemented. Those gaps are filed as
GitHub issues about the code ("Not the docs the code"). And "The manual
should contain only direction on how it SHOULD work. It is our
responsibility to make sure it does": a passage describing intended
behavior the code lacks stays, and the issue makes the code match it. On
doubt: "if it smells wrong to you, we should make it smell right. That's
why a filed issue is important to get it fixed."

Terms. A *reference* is one code span, or one name inside a fenced block,
that the check extracts. An *outcome* is what the check concluded about it.
A *verdict* is the hand-checked classification of an unresolved reference,
with source evidence. The *gap check* is the search, made before a cut, for
evidence that the missing thing was meant to exist or should. A *code gap*
is a finding whose fix belongs in the code.

## The comparison

[MAN-001] What the code does MUST be established by reading the source at
the surface's recorded commit; the manual, the CHANGELOG, a spec, or
anyone's memory of the API MUST NOT establish it. Whether the manual or the
code is wrong where they disagree MUST be decided by the gap check
(MAN-101), not by assuming either side.
Falsifier: A verdict's evidence cites no source location, or cites only the manual or the CHANGELOG for what the code does.
Mechanism: review: every verdict cites source by file and line; `feature-map/tools/manual_triage.py` refuses a verdict without evidence.
Status: Agreed 2026-09-27

[MAN-002] The check MUST run against a surface built from the current
source. When `feature-map/meta.json`'s `source_rev` is not the last commit
that touched the source paths, the check MUST refuse to run.
Falsifier: `manual_check.py` produces findings while `source_rev` lags the source.
Mechanism: `feature-map/tools/manual_check.py` refuses a map stamped behind the source.
Status: Agreed 2026-09-27

[MAN-003] Every English chapter under `manual/` MUST be read through a
CommonMark parser, and every Rust fenced block through tree-sitter-rust.
Every inline code span and every fenced block in `rust`, `bash`, `sh`,
`shell`, `console`, `env`, `dotenv`, `html`, `askama`, `jinja` and
`svelte` MUST yield its references. Fences in any other language are not
checked; that is a known hole, not coverage.
Falsifier: A code reference in a checked context produces no row in the check output.
Mechanism: `feature-map/tools/manual_check.py`.
Status: Agreed 2026-09-27

[MAN-004] Every extracted reference MUST receive exactly one outcome from
the set `manual_check.py`'s docstring defines, and that docstring MUST stay
the single definition of each outcome.
Falsifier: A row carries an outcome outside the set, or a second definition of an outcome disagrees with the docstring.
Mechanism: `feature-map/tools/manual_check.py` (the docstring is the outcome set).
Status: Agreed 2026-09-27

[MAN-005] Every reference with outcome `missing`, `wrong_path` or `hidden`
MUST have a verdict in `manual-triage-verdicts.json`, with a class and
evidence that cites the source by file and line. The triage run MUST fail
while any such reference lacks one.
Falsifier: `manual_triage.py` exits 0 while a `missing`, `wrong_path` or `hidden` finding has no verdict.
Mechanism: `feature-map/tools/manual_triage.py` exits non-zero on an unreviewed reference.
Status: Agreed 2026-09-27

[MAN-006] A change that fixes or removes the manual text a verdict
describes MUST remove that verdict in the same commit. A committed triage
run MUST report no stale verdicts.
Falsifier: `manual_triage.py` reports a non-empty `stale_verdicts` at a committed state.
Mechanism: `feature-map/tools/manual_triage.py` exits non-zero on a stale verdict.
Status: Agreed 2026-09-27

[MAN-007] A problem the check cannot see - a wrong default, a real API
used wrongly, a wrong claim about behavior - MUST be recorded under
`extra` with its own locations, under the same evidence rule as MAN-005.
An `extra` entry does not go stale on its own; the change that fixes it
MUST remove it.
Falsifier: A hand-found manual error is neither fixed nor listed in `extra`.
Mechanism: `feature-map/tools/manual_triage.py` checks `extra` entries under the verdict rules; review finds them.
Status: Agreed 2026-09-27

[MAN-008] The comparison covers the English chapters only. Translations,
`README.md`, rustdoc and the CHANGELOG are outside it, and no report MUST
claim otherwise.
Falsifier: A report or commit message presents a translation or the README as checked.
Mechanism: review.
Status: Agreed 2026-09-27

[MAN-009] The check MUST NOT presume a reference is the reader's own code
without something in the manual backing that presumption. A `Type::member`
reference whose `Type` no Suprnova record has counts as the reader's own
type only when a Rust block somewhere in the manual defines or imports
`Type`; otherwise its outcome is `missing` and it goes to triage.
Falsifier: A `Type::member` reference whose type neither a surface record nor any Rust block in the manual defines or imports receives the outcome `weak_missing`.
Mechanism: `feature-map/tools/manual_check.py` (the MAN-009 pass after the walk).
Status: Agreed 2026-09-27

## Remediation

The manual states how Suprnova should work. A passage that describes
something nobody intended is the manual's error and is cut or corrected. A
passage that describes intended behavior the code lacks is the code's
defect: the passage stays, an issue is filed against the code, and fixing
the code is ours to do. The gap check decides which of the two a finding
is.

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

| Finding | Verdict | Manual | Code |
|---|---|---|---|
| Wrong name or path; the public equivalent exists | `error` or `wrong_path`, `gap: none` | use the real name or path | nothing |
| Does not exist; no evidence it was meant to, and no gap worth filing | `error`, `gap: none` | cut the passage | nothing |
| Does not exist, and the gap check found intent (shipped, agreed, or pointed at by the code's docs) or a real gap (security, a Laravel equivalent that parity claims) | `code_bug`, `gap: issue` | keep the passage | issue; the code is made to match |
| Exists but is broken | `code_bug`, `gap: issue` | keep the passage | issue; the code is made to match |
| Exists, public but `#[doc(hidden)]`, and a reader needs it (MAN-104) | `hidden`, `gap: issue` | keep the passage | issue to make it supported API |

A kept passage may still be corrected where its details are wrong about the
intended behavior; it is not weakened to describe what the code does today.
Where stating the intended behavior would mean naming API the issue has not
settled, the passage keeps its accurate description of the gap, tied to the
issue by its verdict, until the fix names it.

The first row is for a slip: the manual names the right thing wrongly. When
the manual's form is a different shape from anything the code offers, and
it is the form a reader would expect - the code reaches the same result
only another way, or with more ceremony - the finding is treated as
intended: `code_bug`, the passage stays, and the code gets an issue.
Doubt keeps the passage; it never cuts it.

The two columns are two streams of work. The documentation pass changes
only the manual. Issues change only the code, each in its own change
(MAN-105); the pass files them as the gap check finds them, and does not
make them.

[MAN-101] Before a finding is classified `error` or `code_bug`, and before
any manual text is changed for it, the gap check MUST run in the order
listed above, and its result MUST be recorded in the verdict: the evidence
names what was searched, and the `gap` field is `none` or `issue`. Where
the gap check leaves doubt whether the manual's form was intended, the
finding MUST be treated as intended.
Falsifier: An `error` or `code_bug` verdict has no `gap` field, its evidence names no search, or an `error` verdict's evidence records doubt about intent.
Mechanism: `feature-map/tools/manual_triage.py` requires `gap` on `error` and `code_bug` verdicts.
Status: Agreed 2026-09-27

[MAN-102] Each finding MUST be remediated by the table above. The manual
MUST NOT describe anything nobody intended, and a passage the gap check tied
to an issue MUST NOT be cut or rewritten to fit the code while that issue is
open.
Falsifier: A `gap: none` passage remains in the manual, or a passage tied to an open issue is cut or rewritten around the code's current behavior.
Mechanism: review.
Status: Agreed 2026-09-27

[MAN-103] An issue filed under MAN-102 MUST describe the code: its title
names the code's defect or missing capability, and its body gives what
happens or what is missing, the cause with file and line, where the
manual, the CHANGELOG or the code's docs claim it, and a suggested fix. It
MUST NOT ask for a manual change; the documentation pass makes that
change. Existing issues MUST be searched first, and the verdict MUST record
the issue number in `issue`.
Falsifier: An issue's requested change is to the manual, or a `gap: issue` verdict has no `issue` number.
Mechanism: `feature-map/tools/manual_triage.py` requires `issue` when `gap` is `issue`.
Status: Agreed 2026-09-27

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
Mechanism: `feature-map/tools/manual_triage.py` requires `audience` on `hidden` verdicts.
Status: Agreed 2026-09-27

[MAN-105] A remediation change MUST do only what its findings need.
Manual fixes go in the documentation pass; code changes go through their
issue, in their own change, unless the developer rules otherwise. A
documentation pass MUST NOT add features, services or infrastructure.
Falsifier: A documentation-pass commit changes behavior under `framework/`, `crates/`, `suprnova-cli/` or `suprnova-macros/`.
Mechanism: review.
Status: Agreed 2026-09-27

[MAN-106] Every change to the manual MUST re-run the check and the triage
in the same change and commit with no unreviewed findings and no stale
verdicts. A change that also touches source MUST regenerate the surface
and restamp `source_rev` first.
Falsifier: Re-running the check and triage on a committed state disagrees with the committed triage output.
Mechanism: `feature-map/tools/manual_triage.py` exits non-zero on unreviewed findings and stale verdicts.
Status: Agreed 2026-09-27

Claims about the code's state in time are where the manual goes stale
first, and the name check cannot see them: a "Limitations (v1)" section
said eager loads bypass the active transaction and a "Coverage scope"
section said Eloquent reads do not fire `QueryExecuted`, and both had
already been fixed and pinned by regression tests
(`framework/tests/eloquent/relations_tx.rs`,
`framework/tests/eloquent/read_instrumentation.rs`). On 2026-09-27 a line
search for the markers MAN-107 lists matched 193 lines in 58 chapters,
not all of them claims (a bare `v1` is often an API version), and the
developer ruled that day that every such claim requires inline
verification.

[MAN-107] Every claim in the manual about the code's state in time - a
limitation, a gap, a plan, a version-scoped caveat, or a sentence marked
today, for now, currently, not yet, at the moment, follow-up, planned,
known seam, will land, lands in, in a future, yet, v1, or time of writing - MUST be verified against the source as the
pass reaches it, and MUST carry a verdict. A stale claim is the manual's
error: the text is corrected or cut. A true gap the code should close is a
`code_bug` with an issue, and the manual states the intended behavior. A
true constraint that is intended, such as a database's own behavior or a
design choice, stays, stated as how Suprnova works rather than as a
moment in its history. A claim that something is planned MUST stand on a
record of the plan (`docs/spec`, `docs/commitments`, `docs/decisions`, the
CHANGELOG) or the developer's word; the source cannot show intent, and
without either the claim is the manual's error.
Falsifier: A time-marked claim in an English chapter has no verdict, or a verdict that cites no source, or a claim that something is planned is kept with no plan record and no ruling from the developer.
Mechanism: `feature-map/tools/manual_check.py` extracts `time_claim` findings; `feature-map/tools/manual_triage.py` requires a verdict for each.
Status: Agreed 2026-09-27

## Mechanisms

What exists:

- MAN-002, MAN-003, MAN-004, MAN-009: `feature-map/tools/manual_check.py`,
  which refuses a map stamped behind the source.
- MAN-005, MAN-006: `feature-map/tools/manual_triage.py` exits non-zero
  and lists the unreviewed references and the stale verdicts.
- MAN-101, MAN-103, MAN-104: `manual_triage.py` requires `gap` on `error`
  and `code_bug` verdicts, `issue` when `gap` is `issue`, and `audience`
  on `hidden` verdicts.
- MAN-107: `manual_check.py` extracts time-marked prose sentences as
  `time_claim` findings, and `manual_triage.py` requires a verdict for
  each, so a claim cannot pass unverified.
- MAN-001, MAN-007, MAN-008, MAN-102, MAN-105, MAN-106: review.

## Translations

On hold, by the developer's ruling of 2026-09-27: upcoming changes would
make the translation work redundant, so the pass works on the English
manual alone and the locale mirrors are not updated alongside it (MAN-008
already scopes the check to English). `.manual-translations.lock` keeps
recording which English chapters moved past their mirrors, which is the
list to retranslate when the hold lifts.

Make sure the translations gate is disabled until Shawn says to enable
it. Left on, it fails the full local gate on any English edit and blocks
a release on a stale mirror (`docs/recon.md`, `overview.md`); it lives in
the gate scripts on Shawn's machine (`local/gate-infra`), not in this
repository.

When the hold began, one chapter was past its mirrors: `filesystem.md`,
by one table row (the S3-compatible examples added on
`eas4ai/nice-lamport-lhxidv`). At the end of the fix pass, 56 English
chapters on that branch are past their mirrors (none on `main`), which is
the retranslation list when the hold lifts:

`authorization.md`, `broadcasting.md`, `bus.md`, `cli-docker.md`,
`cli.md`, `console.md`, `database.md`, `database-testing.md`,
`data.md`, `deployment-digital-ocean.md`, `deployment-railway.md`,
`development.md`, `eloquent-factories.md`, `eloquent.md`,
`eloquent-relationships.md`, `eloquent-serialization.md`,
`encryption.md`, `env-vars.md`, `events.md`, `filesystem.md`,
`from-laravel.md`, `from-rust-web.md`, `frontend-inertia-responses.md`,
`frontend-pages.md`, `glossary.md`, `hashing.md`, `http-client.md`,
`idempotency.md`, `images.md`, `installation.md`, `introduction.md`,
`live.md`, `logging.md`, `macros.md`, `mail.md`, `mocking.md`,
`notifications.md`, `observability.md`, `parity.md`,
`payments-frontend.md`, `payments.md`, `payments-paddle.md`,
`queries.md`, `queues.md`, `render-cache-generations.md`,
`responses.md`, `seeding.md`, `starter-kits.md`, `structure.md`,
`supervisors.md`, `testing.md`, `timeout.md`,
`tutorial-inertia-crud.md`, `urls.md`, `vector.md`, `websockets.md`.

## Resolved questions

- Framework-contributor material (test and bench environment variables,
  "where the code lives" tables) stays in the manual when accurate;
  MAN-104 still takes the internal names out. Decided 2026-09-27.
