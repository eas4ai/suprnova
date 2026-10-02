# Recon - Suprnova (Sudus, Path B)

Date: 2026-09-30. Surveyed at main `e71e06eb`; `origin/main` is `c6a4ee8b`, and
main carries three unpushed commits (`75670a34`, `a07f7cea`, `e71e06eb`).

Sudus 4.2.8 was initialized on 2026-09-28 with `origin` as the authority
remote (`137d3df3`); the spec was converted to the Sudus block grammar on
2026-09-29 (`125632c2` to `f2254c2e`); the Cairn 1 records left the tree on
2026-09-30 (`a07f7cea`, `e71e06eb`). No commitment has started: `sudus wake`
exits 3. A spec set exists (`docs/spec/overview.md`), so this is Path B: the
recon covers every commit since the newest Agreed date, 2026-09-27 (the MAN
requirements). That is 144 commits. Sol (gpt-6.1-sol, read-only, no builds)
classified each against the spec; every Contradicted row below was then
checked against the files.

This replaces the 2026-09-13 Path A recon (Git history keeps it, last at
`a07f7cea`). Its unresolved findings are carried below.

## Exists

| What | Evidence |
|---|---|
| Cargo workspace, 12 members plus the excluded Live fuzz crate, version 3.0.0, edition 2024, rust-version 1.94.0 | `Cargo.toml:1-22` |
| v3.0.0 released 2026-09-29; work that landed after the tag is filed under the 3.0.0 heading with "landed on main after the `v3.0.0` tag" | tag `v3.0.0` = `360347e4`; `CHANGELOG.md:7`, `:24` |
| 63 integration test binaries under `framework/tests/` | `ls -d framework/tests/*/` |
| The dogfood application mounts 21 Live views, among them one gallery per component family and the datatable | `app/templates/live/` |
| The repository gate has 29 steps and no translation step; `scripts/` is local and untracked | `scripts/gate-steps.json`; `git ls-files scripts` is empty |
| Live's own gate has 22 phases | `crates/suprnova-live/scripts/gate.sh` |
| The manual has 112 English chapters; the six locale mirrors are frozen under the translation hold | `manual/`; `docs/spec/manual-check.md:264-300` |
| The feature map: 11,230 surface records stamped at `c0fa5afb`, 164 triage verdicts; the Laravel map with 21,854 surface and 21,758 parity rows | `feature-map/meta.json`, `feature-map/manual-triage-verdicts.json`, `feature-map/laravel/` |
| Sudus record: settings with attribution forbidden, the log and snapshot refs on origin, one backlog item `chart-follows-the-tokens` (DATA-004), no decisions | `.sudus/settings.json`; `git ls-remote origin 'refs/sudus/*'`; `sudus show items`; `sudus decisions` |
| GitHub: 111 issues closed since 2026-09-27 (#9 to #124, the remediation the developer made top priority on 2026-09-28); 10 open, #125 to #134 | `gh issue list` |

## History since 2026-09-27

144 commits. 56 touch a spec domain; 88 do not.

Outside every spec domain (88), by subsystem: Eloquent and the model macros
11, Magnetar 10, queue and bus 7, data, requests and Inertia 5, test helpers
and feature gates 5, CLI and scaffolding 4, HTTP proxies and rate limits 4,
payments 4, schema 3, routing 3, workflow and errors 3, crypto, database,
console, telemetry, middleware, fakes and the changelog 2 each, and one each
for the container, media, HTTP client, mail, web push, WebSockets,
filesystem, vector, scheduling, broadcasting, pagination, the Sudus
settings, a dogfood import cleanup, a session test's formatting and the
Laravel comparison map (`90baf742`). These are the issue remediation, the
v3.0.0 release and the start of the Laravel parity program; no spec file
covers their domains (`docs/spec/overview.md`, the second table of the spec
map).

Inside a spec domain (56):

- 13 edit the spec: the eleven commits converting it to the Sudus grammar on 2026-09-29, which
  moved metadata into rationale and changed no requirement statement, and
  the two migration commits of 2026-09-30, which cut references to removed
  paths (the note after Contradicted 7).
- 26 leave their requirements holding: Live (LIVE-001, -002, -006, -007,
  -008, -010, -016, -017, -019, -020, -021, -025, -031), RenderCache (CACHE-002, -005,
  -008, -009), sessions (SESS-001), and the component library (FORM-005,
  FORM-007, UI-001, -002, -004, -007, -008, -012), mostly refactors and
  added tests from the remediation and two token fixes (`87a79273`,
  `7d9c3f38`).
- 17 contradict a requirement: 15 edit the manual without regenerating the
  surface or re-running the check (Contradicted 1), and 2 add triage
  verdicts whose evidence cites no source (Contradicted 2).

Runtime behavior of these commits was not exercised for this recon; the
classification reads their diffs.

## Documented

| What | Where |
|---|---|
| Placement (owner rulings 2026-09-13): the record (`docs/spec/`, `docs/recon.md`, `.sudus/`) is tracked; `docs/superpowers/` at any level is a local planning archive and stays ignored | `.gitignore`; Git history of this file |
| The Laravel parity program (developer, 2026-09-30): Laravel 13.34.0 and Inertia 3.7.1, filtered only by whether an item improves Suprnova. Its record is the Laravel map, the rulings the developer gave in the parity viewer (87 open disagreements accepted, the Pusher driver approved, AVIF refused) and a brief with a recommendation for each of 2,540 decision groups (619 build, 743 refuse, 905 not applicable, 98 already shipped, 175 open). No spec file covers it | `feature-map/laravel/`; the parity viewer and brief are artifacts outside the repository |
| Open issues: #125 joins, #126 file responses, #127 handler authorization attributes, #128 query and model helpers, #129 a configurable Inertia page lookup (outside contributors); #130 the Pusher driver (developer-approved, in progress in a worktree); #131 to #134 defects the parity review found (#131 fixed at `75670a34`, unpushed) | `gh issue list` |
| Implementation specs for #130 and for #132 to #134 | `docs/superpowers/specs/2026-09-30-pusher-driver.md`, `2026-09-30-defects-132-134.md` (local archive) |

## Contradicted

Each row cites both sides; the developer rules which side is wrong.

1. **MAN-106: the manual moved without the check.** MAN-106 requires every
   manual change to re-run the check and the triage, regenerating the
   surface first when source changed too. 15 commits after the surface was
   stamped (`988a7535`, `source_rev` `c0fa5afb`) changed the manual,
   among them the v3.0.0 release, the schema builder, the date-time casts,
   the serde naming work and `75670a34`; none regenerated the surface, so
   the check now refuses to run (MAN-002). Both sides:
   `docs/spec/manual-check.md:211-216` against `feature-map/meta.json:10`
   and `git log c0fa5afb..HEAD -- manual`.
2. **MAN-005: verdict evidence without a source location.** MAN-005
   requires each verdict's evidence to cite the source by file and line.
   139 of the 164 verdicts cite no file: most are `noise` verdicts for names
   that are not Suprnova's (SQL keywords, a tokio macro, a clap derive),
   which have no Suprnova source to cite, and `1a20fc25` added two
   (`TenantApiClient::for_current_tenant`, `TenantDb::connect_for`) whose
   evidence is the manual's own example. `manual_triage.py` does not check
   for a citation. Both sides: `docs/spec/manual-check.md:72-78` against
   `feature-map/manual-triage-verdicts.json` and
   `feature-map/tools/manual_triage.py:58-68`.
3. **MAN-006 names a field the tool does not emit.** Its falsifier says the
   triage "reports a non-empty `stale_verdicts`"; the tool reports problems
   of kind `stale` under `failed`. Both sides:
   `docs/spec/manual-check.md:83` against
   `feature-map/tools/manual_triage.py:72,86`.
4. **FORM-005's mechanism names a missing test.** It cites an
   "`app/tests/live_uploads.rs`-style" test; no such file exists, and the
   upload end-to-end tests are `app/tests/avatar_upload_e2e.rs` and
   `app/tests/live_upload_reacquire.rs`. Both sides:
   `docs/spec/component-library-forms.md:57` against `app/tests/`.
5. **The keystone (Observed) names five manual chapters that do not
   exist**: `artisan.md`, `views.md`, `inertia.md`, `magnetar.md`,
   `media.md`. The index names `console.md`,
   `frontend-inertia-responses.md` and `images.md` for three of them. Both
   sides: `docs/spec/overview.md:110-122` against `manual/documentation.md`.
6. **The keystone (Observed) describes the translation ledger as live.** It
   says the full tier checks manual translation currency and that the
   ledger blocks a release on a stale mirror; under the hold the gate has
   no translation step. Both sides: `docs/spec/overview.md:52,70-72`
   against `scripts/gate-steps.json` and `docs/spec/manual-check.md:264-277`.
7. **No Agreed requirement has a Sudus mechanism yet.** Every Agreed block
   names a mechanism by its Cairn 1 name or a test; none is declared under
   Sudus, so `sudus check` can observe nothing today. Their definitions are
   below; 11 of their commands run scripts under `.cairn/tools/`, which
   left the tree at `74b0122f` and are recoverable from `74b0122f^`. This
   is what the migration leaves, not a defect: each commitment declares the
   mechanisms its requirements need.

Two migration edits changed Agreed words, both to remove references to
paths that no longer exist: MAN-101's search order and MAN-107's list of
plan records no longer name `docs/commitments` and `docs/decisions`
(`a07f7cea`), and LIVE-022's falsifier cites its baseline receipt by date
instead of a `.cairn/evidence/` path (`e71e06eb`).

Carried from the 2026-09-13 recon:

- The four guidance rows, the `check-mysql.sh` comment, the published
  `scripts/` and Live planning archive, and the tooling branch: resolved on
  2026-09-13 (its disposition).
- The nine rows of the 2026-09-12 scoping notes against Live specs 20-25:
  resolved in the component-library commitments. Styling, the chart and the
  custom-element tier by the recorded decisions (now in Git history at
  `a07f7cea^:docs/decisions/`); the date picker by FORM-007; the datatable
  by DATA-005; tag input, nested menus, the command palette and complete
  blocks such as the stepper left out of every commitment by the
  developer's 2026-09-12 ruling (`docs/spec/roadmap.md:170-175`).

## Unverified

| What | Why |
|---|---|
| The runtime behavior of the 56 commits inside the spec's domains | Classified from their diffs; no test ran for this recon. The last full repository gate passed on the tree pushed as `c6a4ee8b` |
| Whether the 11 `.cairn/tools/` scripts still run against today's tree | Not exercised; the first commitment that declares one of their mechanisms will show it |
| Whether `sudus check` accepts a mechanism whose command uses the untracked `scripts/` (carried from 2026-09-13, then about `cairn check`) | Not yet exercised |
| The pre-2.0 audits' individual findings (carried) | Re-verification is its own work item if a commitment touches Magnetar |

## Blast radius

Traced once the developer names the work.

## Cairn 1 mechanisms, carried for declare

Recovered 2026-09-30 from Git history (`74b0122f^:.cairn/mechanisms/`, removed from the tree
at `74b0122f` on 2026-09-21). Sudus reads no 1.x record; each line keeps the mechanism's
command, inputs and the requirements it observed, so a commitment that needs one can
`sudus declare` it again. Thread and job counts in the commands are the 1.x values; builds
now cap jobs and test threads at 8.
Eleven commands run a script under `.cairn/tools/`; those scripts went with the same commit
and are recovered from `74b0122f^:.cairn/tools/` when their mechanism is declared again.

- **cache-content-encoding** (CACHE-005)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::content_encoding_replays)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-csp-nonce** (CACHE-004)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::csp_nonce_is_never_replayed)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-head-first** (CACHE-006)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::head_first_does_not_publish_get)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-named-connection** (CACHE-008)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::named_connection_is_preserved)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-no-store** (CACHE-001)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::no_store_is_a_storage_veto)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-request-directives** (CACHE-007)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::request_directives_are_honored)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-security-headers** (CACHE-003)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::security_headers_replay_or_decline)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-snapshot-failure** (CACHE-010)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::snapshot_failure_is_uncacheable)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-vary** (CACHE-002)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::vary_must_match_declared_dimensions)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **cache-write-atomicity** (CACHE-009)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test render_cache -E 'test(hardening::write_and_generation_commit_together)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/render_cache/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **checker-soundness** (LIVE-025, LIVE-026, LIVE-027)
  - command: `sh -c 'env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova-live --test checker_regressions --test binding_metadata -E "test(checker_proof) or test(live_error_target) or test(debounce)" && env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo test -p suprnova-macros --test live_ui'`
  - inputs: `crates/suprnova-live/src/checker/`, `crates/suprnova-live/src/state/`, `crates/suprnova-live/tests/checker_regressions.rs`, `crates/suprnova-live/tests/binding_metadata.rs`, `crates/suprnova-live/tests/checker_support/`, `crates/suprnova-live/tests/fixtures/`, `crates/suprnova-live/fixtures/v4/`, `crates/suprnova-live/components/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `suprnova-macros/src/live/`, `suprnova-macros/tests/live_ui.rs`, `suprnova-macros/tests/ui/live/`, `app/src/live/`, `app/templates/`, `app/tests/`, `manual/`, `CHANGELOG.md`, `.manual-translations.lock`, `Cargo.lock`
- **live-action-transaction** (LIVE-017)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::required_transaction_is_refused_until_real)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-async-issuance-cap** (LIVE-018)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::issuance_cap_holds_under_concurrency)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-async-revocation** (LIVE-016)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::revoked_gate_ends_delivery)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-contracts** (LIVE-013)
  - command: `node crates/suprnova-live/scripts/check-specs.mjs && node crates/suprnova-live/scripts/check-implementation-docs.mjs && crates/suprnova-live/tests/documentation_contract.sh`
  - inputs: `crates/suprnova-live/docs/`, `crates/suprnova-live/scripts/check-specs.mjs`, `crates/suprnova-live/scripts/check-implementation-docs.mjs`, `crates/suprnova-live/tests/documentation_contract.sh`
- **live-gate** (LIVE-010, LIVE-011, LIVE-012, UI-008, UI-012, OVL-005, FDB-004, NAV-003)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 crates/suprnova-live/scripts/gate.sh`
  - inputs: `crates/suprnova-live/`, `framework/src/live/`, `Cargo.lock`
- **live-issuance-credentials** (LIVE-022, LIVE-023)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::concurrent_issuance_keeps_every_credential)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-key-vocabulary** (LIVE-024)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/live-key-identity.test.ts && cd ../../.. && env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova-live --test checker_regressions -E "test(live_key)"'`
  - inputs: `crates/suprnova-live/browser/tests/live-key-identity.test.ts`, `crates/suprnova-live/browser/tests/support/`, `crates/suprnova-live/browser/src/morph/`, `crates/suprnova-live/browser/src/uploads/morph.ts`, `crates/suprnova-live/browser/src/stimulus/lifecycle.ts`, `crates/suprnova-live/browser/src/signals/lifecycle.ts`, `crates/suprnova-live/browser/src/transitions/lifecycle.ts`, `crates/suprnova-live/src/checker/`, `crates/suprnova-live/tests/checker_regressions.rs`, `crates/suprnova-live/components/`, `app/templates/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `manual/`, `CHANGELOG.md`, `.manual-translations.lock`
- **live-library-review-remediation** (FORM-008, FORM-009, FORM-010, FORM-011, FORM-012, FDB-007, NAV-007, OVL-007, DATA-006, UI-020, UI-021, UI-022, UI-023, UI-024, LIVE-031, LIVE-032, LIVE-033, LIVE-034, LIVE-035, LIVE-036)
  - command: `node .cairn/tools/live-library-review.mjs`
  - inputs: `.cairn/tools/live-library-review.mjs`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `crates/suprnova-live/components/`, `crates/suprnova-live/browser/src/`, `crates/suprnova-live/browser/dist/`, `crates/suprnova-live/browser/e2e/`, `crates/suprnova-live/browser/tests/`, `crates/suprnova-live/browser/test-host/`, `crates/suprnova-live/browser/playwright.components.config.ts`, `crates/suprnova-live/browser/playwright.dogfood.config.ts`, `crates/suprnova-live/browser/package.json`, `suprnova-macros/src/live/`, `framework/src/live/`, `framework/src/view/`, `framework/tests/live/`, `framework/Cargo.toml`, `framework/tests/support/`, `framework/tests/templates/live/`, `app/`, `suprnova-cli/src/`, `suprnova-cli/tests/live_add.rs`, `manual/live.md`, `Cargo.lock`
  - also: `results: per-requirement`
- **live-model-render-baseline** (LIVE-037)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/model-render-baseline.test.ts && npx playwright test --config playwright.dogfood.config.ts --grep LIVE-037'`
  - inputs: `crates/suprnova-live/browser/src/models/`, `crates/suprnova-live/browser/src/runtime/`, `crates/suprnova-live/browser/src/morph/`, `crates/suprnova-live/browser/src/continuity/`, `crates/suprnova-live/browser/tests/model-render-baseline.test.ts`, `crates/suprnova-live/browser/tests/support/`, `crates/suprnova-live/browser/dist/`, `crates/suprnova-live/browser/e2e/app-dogfood-forms.spec.ts`, `crates/suprnova-live/browser/playwright.dogfood.config.ts`, `app/`, `CHANGELOG.md`
- **live-protocol-bounds** (LIVE-028, LIVE-029, LIVE-030)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/protocol-bounds.test.ts && cd ../../.. && env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova-live --test checker_regressions -E "test(submit_form_proposals)"'`
  - inputs: `crates/suprnova-live/browser/src/protocol.ts`, `crates/suprnova-live/browser/src/transport/`, `crates/suprnova-live/browser/src/models/`, `crates/suprnova-live/browser/src/runtime/`, `crates/suprnova-live/browser/tests/protocol-bounds.test.ts`, `crates/suprnova-live/browser/tests/request-builder.test.ts`, `crates/suprnova-live/browser/tests/support/`, `crates/suprnova-live/browser/dist/`, `crates/suprnova-live/browser/test-host/`, `crates/suprnova-live/browser/e2e/`, `crates/suprnova-live/src/checker/`, `crates/suprnova-live/tests/checker_regressions.rs`, `crates/suprnova-live/docs/specs/suprnova-live/`, `framework/src/live/runtime.rs`, `app/templates/`, `manual/`, `CHANGELOG.md`, `.manual-translations.lock`
- **live-session-deauthentication** (LIVE-021)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::plain_logout_ends_delivery)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/session/`, `framework/src/auth/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-session-reverification** (LIVE-020)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::stale_store_session_ends_delivery)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/session/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
  - also: `reviewed:`
  - also: `- LIVE-020 sha256:72f635132fe8bbf23eeb8507f89ba6bfabebba19c7d48cc9dbc710fba979e32e`
- **live-session-revocation** (LIVE-019)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E 'test(hardening::revoked_session_ends_delivery)'`
  - inputs: `framework/src/render_cache/`, `framework/src/live/`, `framework/src/session/`, `framework/src/database/`, `framework/src/eloquent/`, `framework/tests/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/tests/`, `Cargo.lock`, `framework/tests/support/`, `framework/tests/templates/`, `framework/tests/database/`, `crates/suprnova-live/docs/specs/suprnova-live/`, `CHANGELOG.md`, `manual/`, `.manual-translations.lock`
- **live-upload-store-flush** (LIVE-038)
  - command: `sh -c 'set -e; env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 cargo test -q -p suprnova-live --test upload_file_provider --no-run; binary=$(env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 cargo test -p suprnova-live --test upload_file_provider --no-run --message-format json 2>/dev/null | node -e "let d=\"\";process.stdin.on(\"data\",c=>d+=c).on(\"end\",()=>{for(const l of d.split(\"\\n\")){if(!l.trim())continue;const m=JSON.parse(l);if(m.target&&m.target.name===\"upload_file_provider\"&&m.executable)console.log(m.executable);}})" | tail -1); for run in $(seq 1 60); do if "$binary" > /dev/null 2>&1; then echo "run $run: ok"; else echo "run $run: failed"; "$binary" 2>&1 | tail -20; exit 1; fi; done'`
  - inputs: `crates/suprnova-live/crates/suprnova-live-test-support/src/`, `crates/suprnova-live/src/upload/`, `crates/suprnova-live/tests/upload_file_provider.rs`
- **session-blocking** (SESS-001)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test session -E 'test(blocking::)'`
  - inputs: `framework/src/session/`, `framework/src/cache/`, `framework/src/routing/`, `framework/src/http/request.rs`, `framework/src/lib.rs`, `framework/src/server.rs`, `framework/tests/session/blocking.rs`, `framework/tests/session/main.rs`, `framework/tests/support/`, `Cargo.lock`
  - also: `reviewed:`
  - also: `- SESS-001 sha256:6051490235d7b5016ffd46ad3ab622f09fb66b337a84bfa96c6bea4e078ecc30`
- **spec-lint** (LIVE-014)
  - command: `node .cairn/tools/spec-lint.mjs docs/spec`
  - inputs: `docs/spec/`, `.cairn/tools/`
- **ui-collection-morph** (DATA-003)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/collection-continuity.test.ts'`
  - inputs: `crates/suprnova-live/browser/src/morph/`, `crates/suprnova-live/browser/tests/`
- **ui-combobox-stale** (FORM-008)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/combobox-stale-results.test.ts'`
  - inputs: `crates/suprnova-live/browser/tests/combobox-stale-results.test.ts`, `crates/suprnova-live/components/combobox/`
- **ui-data-display** (DATA-001, DATA-002, DATA-004)
  - command: `node .cairn/tools/ui-data-display.mjs crates/suprnova-live/components app/templates/live/data-display-gallery.html crates/suprnova-live/browser/src`
  - inputs: `.cairn/tools/ui-data-display.mjs`, `crates/suprnova-live/components/`, `crates/suprnova-live/browser/src/`, `app/templates/live/`
  - also: `results: per-requirement`
- **ui-dogfood-tests** (UI-006, UI-015, FORM-004, FDB-003, DATA-005, FORM-005, NAV-005)
  - command: `sh -c 'env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p app --test live_dogfood && env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live -E "test(library_namespace::)"'`
  - inputs: `app/`, `framework/src/live/`, `framework/src/view/`, `framework/tests/live/`, `framework/tests/support/`, `suprnova-macros/src/live/`, `crates/suprnova-live/src/`, `crates/suprnova-live/components/`
- **ui-elements** (UI-011, UI-018)
  - command: `node .cairn/tools/ui-elements.mjs crates/suprnova-live/components`
  - inputs: `.cairn/tools/ui-elements.mjs`, `crates/suprnova-live/components/`
  - also: `results: per-requirement`
- **ui-feed-status** (FDB-005)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/live-feed-status.test.ts'`
  - inputs: `crates/suprnova-live/browser/tests/live-feed-status.test.ts`, `crates/suprnova-live/browser/src/feedback/`, `crates/suprnova-live/browser/src/islands/`, `crates/suprnova-live/components/live-feed/`, `crates/suprnova-live/components/notification-bell/`
- **ui-feedback** (FDB-001, FDB-002, FDB-006)
  - command: `node .cairn/tools/ui-feedback.mjs crates/suprnova-live/components app/templates/live/feedback-gallery.html`
  - inputs: `.cairn/tools/ui-feedback.mjs`, `crates/suprnova-live/components/`, `app/templates/live/`
  - also: `results: per-requirement`
- **ui-framework-tests** (UI-019)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova --test live_assets`
  - inputs: `framework/src/live/`, `framework/tests/live/assets.rs`, `framework/tests/support/`, `crates/suprnova-live/src/artifacts.rs`, `crates/suprnova-live/browser/dist/`
- **ui-islands** (UI-013)
  - command: `node .cairn/tools/ui-islands.mjs crates/suprnova-live/components app/templates/suprnova-ui`
  - inputs: `.cairn/tools/ui-islands.mjs`, `crates/suprnova-live/components/`, `app/templates/`
- **ui-light-dom** (UI-010)
  - command: `sh -c '! grep -rn "attachShadow" crates/suprnova-live/browser/src crates/suprnova-live/components app/templates'`
  - inputs: `crates/suprnova-live/browser/src/`, `app/templates/`, `crates/suprnova-live/components/`
- **ui-live-add** (UI-017)
  - command: `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=12 RUST_TEST_THREADS=12 cargo nextest run -p suprnova-cli --test live_add`
  - inputs: `suprnova-cli/build.rs`, `suprnova-cli/src/`, `suprnova-cli/tests/live_add.rs`, `crates/suprnova-live/components/`
- **ui-live-check** (LIVE-008, LIVE-009, UI-009, UI-014, UI-016, FORM-001, FORM-002, FORM-003)
  - command: `sh -c 'cd app && cargo run -q -p suprnova-cli -- live:check'`
  - inputs: `app/`, `suprnova-cli/src/commands/live_check.rs`, `suprnova-cli/src/commands/live_tool.rs`, `crates/suprnova-live/src/checker/`, `framework/src/live/`
- **ui-live-native** (FORM-006, FORM-007)
  - command: `node .cairn/tools/ui-live-native.mjs crates/suprnova-live/components app/templates/live/live-native-gallery.html`
  - inputs: `.cairn/tools/ui-live-native.mjs`, `crates/suprnova-live/components/`, `app/templates/live/`
  - also: `results: per-requirement`
- **ui-load-more-morph** (NAV-006)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/load-more-continuity.test.ts'`
  - inputs: `crates/suprnova-live/browser/src/morph/`, `crates/suprnova-live/browser/tests/`
- **ui-navigation** (NAV-001, NAV-002, NAV-004)
  - command: `node .cairn/tools/ui-navigation.mjs crates/suprnova-live/components app/templates/live/navigation-gallery.html`
  - inputs: `.cairn/tools/ui-navigation.mjs`, `crates/suprnova-live/components/`, `app/templates/live/`
  - also: `results: per-requirement`
- **ui-overlays** (OVL-001, OVL-002, OVL-003, OVL-004)
  - command: `node .cairn/tools/ui-overlays.mjs crates/suprnova-live/components app/templates/live/overlay-gallery.html`
  - inputs: `.cairn/tools/ui-overlays.mjs`, `crates/suprnova-live/components/`, `app/templates/live/`
  - also: `results: per-requirement`
  - also: `reviewed:`
  - also: `- OVL-002 sha256:7ef7f0ca268a3e91c4fd96f81815c7c217c29e6e676ae37412ef320cd02ed405`
- **ui-overlays-morph** (OVL-006)
  - command: `sh -c 'cd crates/suprnova-live/browser && npx vitest run tests/overlay-continuity.test.ts'`
  - inputs: `crates/suprnova-live/browser/src/morph/`, `crates/suprnova-live/browser/tests/`
- **ui-tokens** (UI-001, UI-002, UI-003, UI-004, UI-005, UI-007)
  - command: `node .cairn/tools/ui-tokens.mjs crates/suprnova-live/browser/src/styles/suprnova-ui.css app/templates/suprnova-ui crates/suprnova-live/browser/src/styles/suprnova-ui.tailwind.css crates/suprnova-live/components manual`
  - inputs: `.cairn/tools/ui-tokens.mjs`, `crates/suprnova-live/browser/src/`, `app/templates/`, `crates/suprnova-live/components/`, `manual/`, `CHANGELOG.md`, `.manual-translations.lock`
  - also: `results: per-requirement`
- **ui-tooltip-dismissal** (OVL-002, OVL-008)
  - command: `node .cairn/tools/ui-tooltip-dismissal.mjs`
  - inputs: `.cairn/tools/ui-tooltip-dismissal.mjs`, `crates/suprnova-live/components/tooltip/`, `crates/suprnova-live/browser/e2e/components/tooltip.spec.ts`, `crates/suprnova-live/browser/e2e/components/support.ts`, `crates/suprnova-live/browser/playwright.components.config.ts`, `crates/suprnova-live/browser/src/styles/suprnova-ui.css`
  - also: `results: per-requirement`
