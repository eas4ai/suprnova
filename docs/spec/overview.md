# Suprnova - keystone

Status: Observed

This file says what Suprnova is and where each part is specified. It holds
no requirements of its own. Each domain spec owns its prefix. Text in this
spec set marked Observed describes what the code does at the cited
revision; only text the developer has confirmed as Agreed is contract.

Written 2026-09-13 at framework main `31fb0ead` from the code, by an agent
adopting the repository under Cairn. The developer corrects it.

## What Suprnova is

Suprnova is a Laravel-inspired full-stack web framework for Rust: hyper and
Tokio for HTTP, SeaORM for the database, Inertia 3 for the SPA-style
frontends, with Laravel-shaped facades (`Auth::login`, `Cache::remember`,
`Event::dispatch`, `#[model]`, `routes!`) on top. This repository is the
framework, not an application: users scaffold their own applications with
`suprnova new`, and everything here is library, macro, CLI, or the internal
dogfood application.

Suprnova Live is the framework's server-driven interactive layer: a
component is a Rust struct with a typed state, actions, and an Askama view;
the server renders every document completely, mounts components as islands
inside it, and updates each island by morphing its markup after an action.
Navigation is real routing over plain HTTP; no client application owns the
page (Live overview, `crates/suprnova-live/docs/specs/suprnova-live/00-overview.md`).

## What Suprnova is not

- Not a crates.io package. Distribution is a git tag per release; the
  `v<version>` tag is the release. This absorbs API churn without downstream
  SemVer bumps (`README.md`, Distribution model).
- Not an application. The `app/` crate exists to exercise every subsystem
  together and to host end-to-end tests.
- Not a single-page application framework. Live's dynamism is island-scoped
  inside a server-rendered document; Inertia frontends are a separate,
  optional path.

## Components and how they communicate

| Component | Owns | Talks to |
|---|---|---|
| `framework/` (the `suprnova` crate) | ~60 subsystem modules under `framework/src/`; the crate-root re-exports in `framework/src/lib.rs` are the public surface | Applications call it; it drives the engine crates below |
| `suprnova-macros/` | Every proc macro (`#[handler]`, `#[model]`, `routes!`, `#[command]`, `InertiaProps`, `#[workflow]`, `LiveComponent`) | Emits `inventory::submit!` registrations the framework collects at link time |
| `suprnova-cli/` | The `suprnova` binary: `new`, `serve`, `make:*`, `migrate*`, `generate-types`, `live:check`, `live:make`, `live:inspect`, `live:assets`; scaffold templates under `suprnova-cli/src/templates/files/` | Generates applications that depend on the framework by git tag |
| `crates/suprnova-live/` | The Live engine: actions, components, endpoint admission, mounts, snapshots, protocol, uploads, async updates, render cache, view contracts; a TypeScript browser runtime with tracked `dist/`; its own gate, numbered spec set, and budgets | The framework's `framework/src/live/` facade adapts it to Suprnova's request model |
| `crates/suprnova-magnetar/` | Auth-service foundations (primary auth, factor gates, passkeys, OAuth, abuse prevention) | Applications reach it only through the framework API (`init_magnetar`, the `magnetar-oauth` feature) |
| `crates/suprnova-payments-{stripe,paddle,nowpayments}`, `crates/suprnova-web-push/` | Adapter crates behind framework traits | Same git-tag distribution |
| `app/` | The dogfood application: every subsystem wired, seven Live views under `app/templates/live/`, end-to-end tests under `app/tests/` | Runs against the in-tree framework |
| `manual/` | The user manual, 112 markdown files, mirrored 1:1 into six locales under a blob-hash ledger (`.manual-translations.lock`) | Published surface; the translation ledger blocks a release on a stale mirror |

Compile-time registration through the `inventory` crate is the backbone of
"declare it anywhere": models, relations, observers, policies, commands,
services, supervisors, workflows, payments, and data registries each have an
`inventory::collect!` point in `framework/src/` (for example
`framework/src/eloquent/registry.rs`, `framework/src/console/mod.rs`,
`framework/src/container/provider.rs`). Anything with a plausible
alternative backend is a trait with drivers behind it (cache, queue,
filesystem, mail, vector, payments, broadcasting, rate limiting, database);
a new backend is a new driver, never a fork of the surface
(`manual/introduction.md`, design principle 3).

## Verification

The gate is the CI: `scripts/gate.sh` runs `scripts/gate-runner.py` over a
default tier (format, prose-dash and doc-reference greps, Live document
contracts, clippy, the rustdoc link check, workspace tests, database steps
against standing services, scaffold compile checks, Live browser artifact
parity) and a full tier that adds the feature matrix, MSRV, audit, manual
translation currency, release smoke, and the whole Live gate. The pre-push
hook refuses a tip without a green stamp. The scripts are local tooling in
every clone and are not published; the reader of the public tree does not
have them, and the relevant rule is always stated inline in this spec set
rather than cited to them.

Suprnova Live keeps its own gate at `crates/suprnova-live/scripts/gate.sh`
(22 phases: document contracts, spec structure, license inventory, Rust
lints and boundaries, MSRV, fuzz build, browser conformance, tracked
artifact parity, the browser unit suite, and the Playwright matrix on
chromium, firefox and webkit with a real BFCache lifecycle). The repository
gate calls it in the full tier.

## Spec map

Reading order is the order below. A domain without a spec file has no
requirements written yet; its published reference is the manual chapter
named. Live has an in-depth reference of its own that this spec set cites
rather than duplicates.

| Domain | Prefix | File | State |
|---|---|---|---|
| RenderCache (document cache: policy, keys, storage, coherence) | CACHE | render-cache.md | Agreed 2026-09-13; ten requirements from the audit of that day, each refining Live specs 15-18; reference: `crates/suprnova-live/docs/specs/suprnova-live/15-...` to `18-...` and `manual/render-cache.md` |
| Live (engine, facade, browser runtime, tooling) | LIVE | live.md | Observed; reference: `crates/suprnova-live/docs/specs/suprnova-live/` (26 numbered specs, glossary, conventions, iteration contracts 001-006) and `manual/live.md` |
| Component library - foundations (inherited by every commitment) | UI | component-library-foundations.md | Draft; refines Live spec 20 |
| Component library - form and input | FORM | component-library-forms.md | Draft; refines Live spec 21; the first commitment with foundations |
| Component library - navigation | NAV | component-library-navigation.md | Draft; refines Live spec 22 |
| Component library - overlay and disclosure | OVL | component-library-overlays.md | Draft; refines Live spec 23 |
| Component library - feedback and status | FDB | component-library-feedback.md | Draft; refines Live spec 24 |
| Component library - data display and layout | DATA | component-library-data-display.md | Draft; refines Live spec 25; DATA-007 Agreed 2026-10-01 |
| Sessions (concurrent requests on one session) | SESS | sessions.md | Draft; SESS-001 Agreed 2026-09-15 on escalation `form-008` |
| Laravel parity (the developer's rulings on the parity map, one commitment at a time) | PAR | laravel-parity.md | Draft; PAR-001 to PAR-011 Agreed 2026-10-01; PAR-012 to PAR-015 Agreed 2026-10-02; the full list is `feature-map/laravel/parity.jsonl` |
| The manual checked against the code, and remediating what the check finds | MAN | manual-check.md | Agreed 2026-09-27 (delegated by the developer); tooling in `feature-map/`, commands in `feature-map/README.md` |

Domains with no spec file yet, and the manual chapters that are their
published reference:

| Domain | Reference |
|---|---|
| Application boot, config, container, console | `manual/lifecycle.md`, `manual/configuration.md`, `manual/container.md`, `manual/artisan.md` |
| HTTP: routing, middleware, requests, responses, CSRF, CORS | `manual/routing.md`, `manual/middleware.md`, `manual/requests.md`, `manual/responses.md`, `manual/session.md` |
| Views and Inertia frontends | `manual/views.md`, `manual/inertia.md`, `manual/frontend.md` |
| Authentication, authorization, Magnetar | `manual/authentication.md`, `manual/authorization.md`, `manual/magnetar.md` |
| Eloquent ORM, migrations, seeding | `manual/eloquent.md`, `manual/migrations.md`, `manual/seeding.md` |
| Validation and localization | `manual/validation.md`, `manual/localization.md` |
| Cache, RenderCache | `manual/cache.md`, `manual/render-cache.md` |
| Queue, bus, events, schedule, workflow, supervisor | `manual/queues.md`, `manual/events.md`, `manual/scheduling.md` |
| Mail, notifications, broadcasting, web push | `manual/mail.md`, `manual/notifications.md`, `manual/broadcasting.md` |
| Filesystem, media, uploads | `manual/filesystem.md`, `manual/media.md` |
| Payments | `manual/payments.md` |
| Vector stores | `manual/vector.md` |
| CLI and scaffolding | `manual/installation.md`, `manual/artisan.md` |
| Testing surfaces | `manual/testing.md`, `manual/http-tests.md`, `manual/database-testing.md`, `manual/mocking.md` |
| Release and distribution | `README.md` (Distribution model), `CHANGELOG.md` |

Manual chapter names above are the published reference for each domain;
where a chapter is named differently in `manual/documentation.md`, that
index wins.

## Technology choices with recorded reasons

- Git tags instead of crates.io: absorb pre-1.0 API churn without SemVer
  bumps downstream (`README.md`).
- `inventory` for registration: declare-anywhere ergonomics; the cost is
  that only linked code registers, so an unreachable module registers
  nothing (framework module docs).
- Askama for Live views: compile-time templates that the checker can prove
  directive by directive (`manual/live.md`, Views).
- Idiomorph as the browser runtime's one dependency; Stimulus 3.2 as an
  optional bridge role the application supplies itself
  (`framework/src/live/assets.rs:129-134`; Live conventions, pinned
  dependencies).
- Tailwind CSS 4 and semantic theme tokens for the official component
  library, with the Live runtime itself CSS-agnostic: decided 2026-08-21 in
  `crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md`
  (Decisions and revisions). The developer reopened the styling question on
  2026-09-13; see `component-library-foundations.md`.

## Status of this specification

Every file carries a `Status:` line; a requirement's own `Status:` line
overrides its file's. Observed text is not contract. The loop refuses a
commitment that names an Observed or Draft requirement until the developer
confirms its text and falsifier as Agreed.
