# Suprnova Live

Status: Observed
Prefix: LIVE

Observed at framework main `31fb0ead`, 2026-09-13. Each requirement below
states what the code does, with the evidence cited; none is contract until
the developer confirms it. Live's own normative reference is
`crates/suprnova-live/docs/specs/suprnova-live/` (26 numbered domain
specs, `glossary.md`, `conventions.md`, `ux.md`, iteration contracts
`iterations/001.md` to `006.md`); this file records only what the
component-library work rests on.

## Documents, islands, and endpoints

[LIVE-001] The framework MUST serve a Live document as a complete
server-rendered HTML response to a plain GET, with each island's markup and
signed snapshot inline.
Falsifier: a Live document's GET response omits an island's rendered
markup and requires the runtime to fetch it.
Evidence: `manual/live.md`, Documents and islands; `framework/tests/live/document_routes.rs`.

[LIVE-002] The framework MUST route Live actions, uploads, and asynchronous
updates under the reserved `__live/` namespace without a version segment
(`__live/action`, `__live/upload`, `__live/async/subscriptions`,
`__live/async/memberships`, `__live/async/events`, `__live/async/socket`).
Falsifier: a Live endpoint is registered under a path that carries a
version segment or lies outside the `__live/` namespace.
Evidence: `framework/src/live/routes.rs:20-21`, `framework/src/live/async_updates.rs:60-66`, commit `31fb0ead`.

[LIVE-003] The framework MUST serve every runtime feature artifact under
`__live/assets/{identity}/{file}` with immutable caching, strong
validators, and integrity attributes in the bootstrap tags. The framework
MUST NOT emit inline script in a Live document.
Falsifier: a document's bootstrap markup contains an inline script, or an
artifact URL lacks the manifest-derived identity segment.
Evidence: `framework/src/live/assets.rs:1-8,30-33`; `manual/live.md`, Assets and no-build use; `framework/tests/live/assets.rs`.

[LIVE-004] The framework MUST load the Stimulus bridge only when a
document opts in through `LiveBootstrapOptions::with_stimulus`. The
application MUST supply Stimulus itself.
Falsifier: a document without the opt-in loads the stimulus artifact role.
Evidence: `framework/src/live/assets.rs:129-134`; `crates/suprnova-live/src/artifacts.rs:93-95`.

## Components and the registry

[LIVE-005] An application MUST register every Live component explicitly
through `LiveRegistry::builder().register::<T>()` before the runtime
assembles; the registry is immutable afterwards.
Falsifier: a component reachable only through link-time inventory, with no
explicit registration, serves an action.
Evidence: `manual/live.md`, Registration and bootstrap; `crates/suprnova-live/src/registry/builder.rs:29`.

[LIVE-006] The registry MUST fail with a typed `RegistryError` on a
duplicate component name, a duplicate view, or a component whose actions
need validation without a validation port.
Falsifier: two components register the same name and the registry builds.
Evidence: `manual/live.md`, Registration and bootstrap; `crates/suprnova-live/src/registry/error.rs:15-17`.

[LIVE-007] A component's `#[action]` methods MUST be the only entry points
the browser can invoke.
Falsifier: a browser request invokes a method not attributed `#[action]`.
Evidence: `manual/live.md`, Authoring a component; `framework/tests/live/public_seed_actions.rs`.

## The checker

[LIVE-008] The checker MUST fail on an unknown action, an unknown model
field, a raw `safe` filter, or an accessibility violation. The checker
MUST report the file, line, and column of each finding.
Falsifier: a registered view references an action the component does not
declare and the checker reports clean.
Evidence: `manual/live.md`, Views; `suprnova-cli/src/commands/live_check.rs`.

[LIVE-009] The checker MUST fail on an unproved dynamic structure unless
the caller passes `--allow-unproved`.
Falsifier: a view with an unproved structure passes the checker without
the flag.
Evidence: `suprnova-cli/src/commands/live_check.rs:145-147`.

## Browser runtime, artifacts, and qualification

[LIVE-010] The Live gate MUST rebuild the browser artifacts from the pinned
lockfile and fail when the tracked `dist/` differs from the rebuild.
Falsifier: an edited tracked artifact whose source did not change passes
the gate.
Evidence: `crates/suprnova-live/scripts/gate.sh:145` (tracked artifact parity); `crates/suprnova-live/browser/package.json`.

[LIVE-011] The Live gate MUST run the browser matrix on chromium, firefox,
and webkit at the pinned Playwright version.
Falsifier: the matrix runs on fewer than the three engines and reports
qualified.
Evidence: `crates/suprnova-live/browser/playwright.config.ts:58-62`; `crates/suprnova-live/browser/package.json:73`; `crates/suprnova-live/scripts/gate.sh:199-214`.

[LIVE-012] The runtime's one production dependency MUST remain Idiomorph;
Stimulus is an optional bridge role.
Falsifier: the core artifact bundles a second runtime dependency.
Evidence: `crates/suprnova-live/browser/src/vendor/`; Live conventions, pinned dependencies (`crates/suprnova-live/docs/specs/suprnova-live/conventions.md:409`).

## Live's own specification discipline

[LIVE-013] The Live spec directory MUST hold exactly the 26 numbered domain
specs plus `conventions.md`, `glossary.md`, and `ux.md`, with iteration
contracts as `iterations/NNN.md`. The Live gate MUST fail on a missing or
extra file there.
Falsifier: a numbered spec is removed, or a file is added beside them, and
`crates/suprnova-live/scripts/check-specs.mjs` reports clean.
Evidence: `crates/suprnova-live/scripts/check-specs.mjs:8-36`; `scripts/check-live-contracts.sh:30` (local tooling).

[LIVE-014] A change to Live's agreed behavior MUST be recorded as a dated
entry in the owning spec's "Decisions and revisions" section before the
code changes.
Falsifier: Live code diverges from a numbered spec's capability text and
that spec carries no dated revision explaining it.
Evidence: every numbered spec's closing section, for example `crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md:179-184`; `iterations/006.md:115-120`.

## What does not exist yet

[LIVE-015] No official component library exists in the code at the
observed revision: no crate under `crates/` provides one, no macro or
stylesheet ships, and the only Live views are the dogfood application's
seven under `app/templates/live/` and the scaffold templates under
`suprnova-cli/src/templates/files/backend/live/`.
Falsifier: a library crate, macro set, or shipped stylesheet is found at
`31fb0ead`.
Evidence: `crates/` listing; `app/templates/live/`; Live specs 20-25 are capability text with no implementation checkpoints.

## Findings of the 2026-09-13 adversarial audit

Drawn from ASTRA-01, ASTRA-05, and ASTRA-07 (Astra, report outside the
repository). Each refines agreed text in Live spec 14 or the action
contract in spec 04; the developer agreed the three, with CACHE-001 to
CACHE-010, as one set on 2026-09-13.

[LIVE-016] The framework MUST re-evaluate the current authorization
(the Gate, the session, and revocation state) before delivering an
asynchronous event to an existing membership. The framework MUST retire
a membership whose authorization no longer holds.
Falsifier: a subscriber whose stream Gate is redefined to deny still
receives an event published after the denial (ASTRA-01).
Mechanism: `.cairn/mechanisms/live-async-revocation`.
Refines: Live spec 14, admission "rechecks ... registry and revocation
state" at the consumption boundary.
Status: Agreed 2026-09-13

[LIVE-017] The framework MUST run an action declared
`transaction = "required"` inside one ambient database transaction that
its ORM writes join. The framework MUST roll that transaction back when a
later stage of the action fails. Until the framework can do both, the
registry MUST refuse a component whose action declares the policy.
Falsifier: an action with two writes whose second stage fails leaves the
first write durable (ASTRA-05).
Mechanism: `.cairn/mechanisms/live-action-transaction`.
Refines: the `transaction` policy in
`crates/suprnova-live/docs/specs/suprnova-live/04-actions-and-validation.md`.
Status: Agreed 2026-09-13

[LIVE-018] The framework MUST reserve a subscription slot under the
per-scope limit before awaiting external authorization. The framework
MUST release that slot on every error path.
Falsifier: 513 concurrent issuances for one scope against a delayed
authorizer all succeed (ASTRA-07).
Mechanism: `.cairn/mechanisms/live-async-issuance-cap`.
Refines: Live spec 14, bounded per-scope issuance.
Status: Agreed 2026-09-13
[LIVE-019] The framework MUST retire every async membership issued to a
session when that session is destroyed on the same node, before any
event published afterwards is appended: session invalidation, session id
regeneration, and `destroy_for_user` each revoke the memberships that
carry the destroyed session's fingerprint or the affected principal.
Falsifier: a subscriber logs out with `Auth::logout_and_invalidate` and
still receives an event published after the logout (the open clause of
LIVE-016).
Mechanism: `.cairn/mechanisms/live-session-revocation`.
Refines: Live spec 14, admission "rechecks ... registry and revocation
state"; LIVE-016 session and revocation-state clauses.
Status: Draft

[LIVE-020] The framework MUST re-verify a membership's session against
the shared session store before delivery, at most once per membership per
ten seconds, and MUST retire a membership whose session the store no
longer holds, so a session destroyed on another node stops receiving
events within that interval.
Falsifier: a session row is removed from the store directly, the clock
advances past ten seconds, and a publish still reaches the membership.
Mechanism: `.cairn/mechanisms/live-session-reverification`.
Refines: Live spec 14, admission "rechecks ... registry and revocation
state"; LIVE-016 session and revocation-state clauses.
Status: Draft
