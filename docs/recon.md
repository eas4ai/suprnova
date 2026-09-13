# Recon - Suprnova (Cairn adoption, Path A)

Date: 2026-09-13. Surveyed at framework main `31fb0ead` (clean tree,
main = origin/main). No spec set existed (`docs/spec/` absent, `cairn wake`
reported "not a Cairn repository"), so this is Path A: Observed specs and a
first commitment get written from the code.

Placement (owner rulings, 2026-09-13 08:52-09:12): the Cairn record - this
file, `docs/spec/`, `docs/commitments/`, `docs/decisions/`, and `.cairn/` -
is tracked in the framework repository; Cairn is tooling under the owner's
rule that development artifacts never reach the public repository.
`docs/superpowers/` at any level is a planning archive and stays ignored
(the root one is its own local git repository). The old nested `docs/`
repository was removed.

The work this session prepares (owner, 08:52): the Live component library.
Its blast radius is traced in the last section.

## Exists

One row per workspace member plus the tooling surfaces. Depth follows the
work; rows outside the blast radius stay one line.

| What | Evidence |
|---|---|
| Cargo workspace, 13 members, version 2.0.1, edition 2024, rust-version 1.94.0 | `Cargo.toml:1-22` |
| `framework/` - the `suprnova` crate: ~60 subsystem modules (auth, cache, eloquent, http, inertia, live, queue, routing, validation, ...) | `framework/src/` listing; `framework/src/lib.rs` (crate-root re-exports) |
| `framework/src/live/` - the framework's Live facade: routes (`/__live/action`, `/__live/upload`, `/__live/async/*`), reviewed-artifact serving under `/__live/assets/*` with integrity metadata and an optional Stimulus role | `framework/src/live/routes.rs`, `framework/src/live/assets.rs:30-135` |
| `suprnova-macros/` - all proc macros (`#[handler]`, `#[model]`, `routes!`, ...) | `suprnova-macros/src/` |
| `suprnova-cli/` - the `suprnova` binary + scaffolding templates, including the Live verbs `live:check`, `live:make`, `live:inspect`, `live:assets` and the Live scaffold templates | `suprnova-cli/src/commands/live_check.rs`, `suprnova-cli/src/commands/live_make.rs`, `suprnova-cli/src/templates/files/backend/live/` |
| `app/` - internal dogfood application; SeaORM migrations under `app/src/migrations/`; seven Live views (counter, todos, dashboard, activity feed, avatar uploader, me, public) | `app/src/migrations/`, `app/templates/live/` |
| `crates/suprnova-live` - the Live engine: 22 submodules (action, component, endpoint, mount, protocol, render_cache, snapshot, upload, view, ...), a browser runtime in strict TypeScript with tracked `dist/`, its own gate, spec set, and budgets | `crates/suprnova-live/src/lib.rs`, `crates/suprnova-live/browser/src/`, `crates/suprnova-live/scripts/gate.sh:44-230` (22 phases), `crates/suprnova-live/benchmarks/*-budget-v1.json` |
| `crates/suprnova-magnetar` - auth-service foundations, consumed only through the framework API | `crates/suprnova-magnetar/` |
| Adapter crates: payments (stripe, paddle, nowpayments), web-push | `Cargo.toml:7-10` |
| Tests: 62 integration test binaries under `framework/tests/` (one per module folder), in-source unit tests, app end-to-end tests, Live's Rust and Playwright suites (pinned `@playwright/test` 1.62.1, chromium + firefox + webkit) | `framework/tests/`, `crates/suprnova-live/browser/package.json:56-73` |
| CI is local: `scripts/gate.sh` -> `scripts/gate-runner.py` (30 full-tier steps), enforced by `.githooks/pre-push`; GitHub auto-runs disabled. The scripts are local tooling in every clone, not published; the install record pins them to a commit on the `local/gate-infra` branch | `.githooks/pre-push`, `scripts/gate-steps.json`, `scripts/tests/test_repository_tracking.py` |
| Release: `scripts/release.sh` (only the owner runs it); distribution is git tags, not crates.io; v2.0.1 released 2026-09-12 (`0db88829`) | `git log` |
| Manual: 112 markdown files in `manual/`, mirrored 1:1 into six locales via a blob-hash ledger; `manual/live.md` is the Live chapter | `manual/`, `.manual-translations.lock`, `manual/live.md` |
| Recent history: v2.0.1 release day, lock hygiene `af442c7f`, Live endpoint de-versioning `31fb0ead` (removed `/v1` from `/__live/*` paths) | `git log --oneline -15` |
| Branches: local only (bench/*, historical feature/release branches, `local/gate-infra` tooling); origin holds main | `git branch -a` |
| Code hygiene: exactly one TODO-shaped string in `framework/src`, and it is a doc-comment example, not a marker | `framework/src/localization/functions.rs:8` |

## Documented

| What | Where |
|---|---|
| **Live's normative spec set** - 26 numbered domain specs, glossary (Live's vocabulary wins over any prior), conventions, six agreed iteration contracts. Specs 20-25 already specify the component library at capability level: foundations (Tailwind CSS 4 styling contract, theme-token contract, anatomy/variants/states, accessibility baseline, catalog), forms, navigation, overlay/disclosure, feedback/status, data display/layout. Decision recorded 2026-08-21: "Official components use Tailwind CSS 4 and semantic theme tokens; the Live runtime remains CSS agnostic." Iteration 006 (agreed 2026-09-08) names specs 20-25 as "the official component-library iteration", sequenced after it | `crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md:179-184`, `21-...` through `25-...`, `00-overview.md:144-146,199,370,416-417`, `conventions.md:210-211,386`, `glossary.md:791-795` (Theme token), `iterations/006.md:96-98` |
| Live implementation docs (27), heading-checked by the gate | `crates/suprnova-live/docs/implementation/` |
| User-facing behavior of every subsystem, Live included | `manual/` (published), `manual/live.md` |
| Changelog, keep-a-changelog, one section per release | `CHANGELOG.md` + six locale mirrors |
| Local assistant guidance (working agreement, commands, architecture, house rules, build rules) - gitignored at every level, not published; the root working-agreement file also carries the Cairn working agreement appended verbatim from the template | not cited: the reader of the published tree has no such files |
| Pre-Cairn audits and the 2026-09-12 library scoping notes (inventory of 58 built-in types, scoping report) | `docs/superpowers/` (local archive, gitignored) |

## Contradicted

Every row is a drift finding, cited on both sides. The developer rules
which side is wrong; the disposition below records the rulings given.

### Guidance and tooling vs code (all resolved 2026-09-13)

| Claim | Code | Both sides |
|---|---|---|
| Local guidance: MSRV is "Rust 1.91.1" | MSRV is 1.94.0 | guidance file (local) vs `scripts/check-msrv.sh:5` and `Cargo.toml` `rust-version = "1.94.0"` |
| Local guidance: the audit policy "is down to one entry (`rsa`)" | Two entries: `rkyv` (expires 2026-11-14) and `rsa` (expires 2027-01-31) | guidance file (local) vs `.cargo/audit.toml:27-51,55-114` |
| Local guidance: Postgres gate tests "run in a throwaway container" | The gate provisions nothing at run time; database steps run against standing services (owner ruling 2026-09-12) | guidance file (local) vs commit `5fb5e851` |
| Local guidance: "never cap `CARGO_BUILD_JOBS`" | Owner standing rule since 2026-09-12: every build capped at 12 threads (`CARGO_BUILD_JOBS=12`, `taskset -c 0-22:2`); benchmarks alone get the whole machine | guidance file (local) vs the owner's 2026-09-12 ruling |
| `check-mysql.sh` comment: "nothing outside `suprnova_test%` is reachable with the gate user's grants" | Grants cover the `suprnova_%` and `magnetar_%` namespaces | `scripts/check-mysql.sh:19-21` vs `scripts/setup/gate-mariadb-user.sql:8-11` |
| `.gitignore:64-66`: "Local tooling - the gate, release scripts, git hooks, and the cargo-audit policy. Not published." | `scripts/` (37 files), `.githooks/pre-push`, `.cargo/audit.toml` were force-added on 2026-08-31 (`7c690e0a`) and pinned in the index by a test; both v2.0.x tags shipped them on the public origin | `.gitignore:64-66` and owner ruling `edffed13` (2026-08-17) vs `git ls-tree origin/main scripts` |
| Root `docs/superpowers/` kept local as a planning archive | `crates/suprnova-live/docs/superpowers/` (14 planning files from Live's iterations 001-004) was published on origin; it arrived tracked with the subtree | `.gitignore` vs `git ls-tree origin/main crates/suprnova-live/docs/superpowers` |
| The `local/gate-infra` tooling branch is the install source | Its `scripts/` lagged main's copies (13 files differed, 6 missing) | `git diff local/gate-infra main -- scripts` at 2c2dd5a1 |

Disposition. Owner, 08:52: update the local guidance so that there is no
drift - the four guidance rows were fixed on the guidance side; the code
was right.
The comment row was fixed in the script. Owner, 09:03: "scripts should not
be in the public github repo", and 09:11 in general form: no development
artifacts in the public repository; tooling (Cairn included, 09:12) may be
published - `scripts/` and the Live planning archive were untracked (files
stay in every clone; history keeps them until the owner orders otherwise),
`scripts/tests/test_repository_tracking.py` now asserts the opposite of
what it asserted before, and the ignore rule covers `docs/superpowers/` at
any level. `.githooks/`, `.cargo/`, `.github/workflows/ci.yml` are tooling
and stay tracked. Live's `docs/specs/` and `docs/implementation/` stay:
the gate reads them (`scripts/check-live-contracts.sh:30-32`). The
tooling branch was synced from main plus today's edits (`162c009f` on
`local/gate-infra`) and the install record re-pinned to it.

### The 2026-09-12 scoping notes vs Live's agreed specs (open, developer rules)

The scoping notes were written without reading specs 20-25; the developer
caught the first of these himself at 08:52 ("you added some things that I
don't agree with and never specified such as 'no tailwind'").

| Scoping note (2026-09-12, local archive) | Live spec (agreed 2026-08-21) | Both sides |
|---|---|---|
| "No Tailwind, no build step" (withdrawn 2026-09-13; the note now records the styling system as an open choice) | Official components use Tailwind CSS 4 utilities and versioned semantic theme tokens; the runtime stays CSS-agnostic; the supported Tailwind range and build configuration are documented and tested | scoping report, Styling section vs `20-component-library-foundations.md:43-67,179-182`, `conventions.md:210-211` |
| Tag input is OUT of the built-in set | The combobox capability includes "tag/token input" with multi-value token keyboard navigation | inventory, rulings header vs `21-form-and-input-components.md:113-128` |
| Command palette is OUT (separate project) | "Command and navigation discovery" is a capability of the navigation domain (MAY-shaped: "may search permitted destinations") | inventory vs `22-navigation-components.md:151-171` |
| Stepper/wizard is a complete block (separate project) | "Steps and progress navigation" is a capability of the navigation domain | inventory vs `22-navigation-components.md:128-149` |
| Dropdown menus are single-level, no nested submenus | Menus: "submenu behavior ... complete" | inventory vs `23-overlay-and-disclosure-components.md:38-53` |
| Chart is a built-in type, server-rendered SVG from Askama, no client charting library | The library supplies "visualization containers and integration contracts, not a new charting runtime"; chart containers "label third-party controller ownership" | inventory vs `25-data-display-and-layout-components.md:158-181,199-200` |
| Date picker is a custom horizontal scroll-snap strip design | Date controls "prefer native semantics where adequate"; custom presentation must define locale, timezone, parsing, constraints, keyboard | inventory vs `21-form-and-input-components.md:136-150,192-193` |
| Datatable builds last; no interactive grid mentioned | Table is the default; an "interactive data grid" is a separate explicit opt-in pattern | inventory vs `25-data-display-and-layout-components.md:84-135,197-198` |
| Widget layer is light-DOM custom elements on an Elena-class helper | "Component-specific JavaScript is implemented through Live local primitives or supported Stimulus controllers" | inventory, widget-layer paragraph vs `20-component-library-foundations.md:110-111` |

These are not fixed here. The spec is the newer party's to revise or the
note's to yield; each row needs the developer's ruling and, where the spec
changes, a dated entry in that spec's "Decisions and revisions" section
(Live's own supersession convention) before the Cairn commitment names the
requirement.

## Unverified

| What | Why |
|---|---|
| The pre-2.0 audits' individual findings against today's tree | All predate v2.0.0; re-verification is its own work item if a commitment touches Magnetar |
| Whether `cairn check` accepts a mechanism whose command lives in gitignored `scripts/` | Not yet exercised; the first mechanism declaration will show it. The Live crate's own scripts under `crates/suprnova-live/scripts/` are tracked and carry no such question |
| Anchor positioning and popover-open continuity across morphs in the three pinned engines | The scoping report's one prerequisite spike; not run |

## Blast radius of the work

The component library touches, by evidence:

- Live's spec set: `crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md` through `25-data-display-and-layout-components.md`, `conventions.md:210-211,386,409`, `glossary.md` (the vocabulary), `00-overview.md:144-146,199,370`, and a new `iterations/007.md` if the build follows Live's contract convention. The gate checks their structure (`crates/suprnova-live/scripts/check-specs.mjs`) and the implementation docs' headings (`crates/suprnova-live/scripts/check-implementation-docs.mjs`).
- Views and the checker: `crates/suprnova-live/src/view/` (contract, document, island, root, trusted HTML), `crates/suprnova-live/src/checker/`, `suprnova-cli/src/commands/live_check.rs`.
- Browser runtime and assets: `crates/suprnova-live/browser/src/` (directives, signals, morph, stimulus, feedback), the tracked `dist/` and its parity check (`scripts/check-live-browser.sh`), `framework/src/live/assets.rs` (how a reviewed asset is served and bootstrapped), the byte budgets `crates/suprnova-live/benchmarks/expansion-budget-v1.json` and `crates/suprnova-live/scripts/check-expansion-budget.mjs`.
- Scaffolding: `suprnova-cli/src/templates/files/backend/live/` and `suprnova-cli/tests/scaffold_snapshot`.
- Dogfood venue: `app/templates/live/`, `app/tests/live_*.rs`.
- Manual: `manual/live.md` and its six locale mirrors under the translation lock (`.manual-translations.lock`); any English edit blocks the full gate until the mirrors are re-stamped.
- Gate: the three Live steps `scripts/check-live-contracts.sh`, `scripts/check-live-browser.sh`, `scripts/check-live-gate.sh` (which runs Live's own 22-phase gate).

Everything outside this list is one line in the keystone's spec map.
