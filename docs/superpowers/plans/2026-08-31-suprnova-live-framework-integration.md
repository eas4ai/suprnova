# Suprnova Live Framework Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Use superpowers:test-driven-development for every behavioral change and superpowers:verification-before-completion before any completion claim. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Suprnova Live a first-class internal Suprnova capability through documented `suprnova::live` and `suprnova::view` APIs, production-owned macros, real framework routes, deterministic runtime assets, CLI tooling, and a generated dogfood application without exposing or duplicating the host-neutral engine.

**Architecture:** Preserve `suprnova-live` as the host-neutral internal engine and make the public framework depend inward on it. `framework/src/live/` owns the curated public facade, immutable application registration, request-context normalization, provider adapters, route installation, response translation, runtime assets, and testing helpers. `framework/src/view/` owns the checked rendering facade. Production macros live only in `suprnova-macros` and expand through `::suprnova::live::__private`. Suprnova middleware remains authoritative for transport and security facts; the adapter converts validated facts into engine capabilities but never runs a parallel Live state machine. RenderCache remains a separate Iteration 005 implementation plan.

**Tech Stack:** Rust 2024, Suprnova router/middleware/container/session/auth/authorization/storage/broadcasting/telemetry APIs, host-neutral `suprnova-live`, Askama checked templates, proc-macro2/quote/syn, deterministic embedded ESM and classic-script assets, Clap CLI, trybuild, Tokio, and the existing Suprnova/Live gate suites.

---

## Non-negotiable boundaries

- Applications and generated examples name only `suprnova`, `suprnova::live`, and `suprnova::view`; they never import `suprnova_live`, the development macro crate, Askama internals, or browser implementation modules.
- The engine keeps no dependency on `framework`; dependency direction is `framework -> suprnova-live` and `suprnova-macros -> proc-macro support only`.
- Framework middleware owns body caps, session/CSRF/origin/auth/tenant/rate facts. Engine validators still fail closed over the resulting typed candidate. Neither side is bypassed.
- The endpoint, upload, and asynchronous engine services remain the only Live protocol/state machines. Framework code adapts their typed requests and responses.
- Reserved Live control paths are versioned and collision-checked. Application-owned upload reacquisition stays outside the reserved namespace.
- Shipped runtime bytes come from the reviewed deterministic artifact manifest. Framework integration never rebuilds, rewrites, or substitutes production JavaScript.
- No RenderCache policy, entry codec, dependency generation, cache store, or stitching implementation enters this plan.
- No component-library, theme, SPA navigation, Alpine/Lit/HTMX migration, generic RPC, streamed trusted HTML, or alternate no-JavaScript action path enters this plan.

## Required ownership flows

```text
request
  -> Suprnova session / CSRF+origin / auth / rate / tenant middleware
  -> crate-private request attestation (each successful owner mints one proof)
  -> Live context adapter consumes exact route requirements
  -> engine validates catalog, scope, proof disposition, lifetime, capabilities
  -> engine service executes
  -> Suprnova translates complete typed HTTP intent
```

Application code cannot construct or mutate the attestation. Missing middleware,
wrong ordering, an excepted branch, a short-circuit, or a principal/tenant change
leaves the corresponding proof absent and the Live adapter fails before engine or
application work.

```text
upload control -> engine ledger/grant -> host quarantine provider -> bounded bytes
  -> engine validation coordinator -> host media/scanner/application validators
  -> engine Ready evidence -> action proposes signed ready handle -> host finalizer
  -> cleanup/cancellation retires ledger, evidence, and quarantine bytes
```

```text
document renderer -> typed artifact role requirements -> exact manifest entries
  -> ESM or classic bootstrap tag + integrity/CSP metadata -> browser deduplicates
  -> runtime compatibility/capability selection -> optional role imports
  -> Live islands connect while server-rendered content remains visible
```

```text
suprnova CLI (no framework dependency)
  -> starts generated application's hidden Live tooling helper
  -> bounded JSON-lines protocol v1 over stdio
  -> framework/runtime performs check, inspect, or asset export
  -> CLI validates version/schema/length/redaction and writes atomically if needed
```

## File structure

- `framework/src/live/mod.rs` - documented public facade and curated re-exports.
- `framework/src/live/config.rs` - validated application configuration and provider selection.
- `framework/src/live/registry.rs` - immutable application component/mount registration and boot-time collision checks.
- `framework/src/live/runtime.rs` - bootstrapped service graph shared by route handlers.
- `framework/src/live/context.rs` - trusted Suprnova request facts to engine candidate/capability conversion.
- `framework/src/live/attestation.rs` - crate-private, request-carried, non-forgeable middleware evidence.
- `framework/src/live/tenant.rs` - configured tenant resolver and route-scoped tenant attestation middleware.
- `framework/src/live/response.rs` - engine HTTP intent to canonical `HttpResponse` conversion.
- `framework/src/live/routes.rs` - reserved HTTP, SSE, and WebSocket route installation.
- `framework/src/live/upload.rs` - Suprnova storage/scanner/authorization adapters for upload control, data, and reacquisition.
- `framework/src/live/async_updates.rs` - authorization, credential, registry, continuity, SSE, and WebSocket adapters.
- `framework/src/live/assets.rs` - exact typed production manifest and embedded artifact responses.
- `framework/src/live/testing.rs` - public request/component/transport test harnesses and safe inspection types.
- `framework/src/live/tooling.rs` - bounded versioned subprocess protocol used by the dependency-light CLI.
- `framework/src/live/__private.rs` - doc-hidden macro ABI re-exports only.
- `framework/src/view/mod.rs` - documented checked view facade and canonical document response helper.
- `suprnova-macros/src/live/` - sole production Live macro implementation.
- `suprnova-macros/tests/ui/live/` - retained pass/fail diagnostics compiled through the real facade.
- `suprnova-cli/src/commands/live_*.rs` - scaffold, check, inspect, and asset workflows.
- `suprnova-cli/src/templates/files/backend/live/` - semantic Live component/view/registration templates.
- `framework/tests/live_*.rs` - facade, adapter, route, hostile-input, asset, and dogfood integration evidence.
- `suprnova-cli/tests/live_*.rs` - dry-run, conflict, invalid-name, scaffold, checker, inspect, and asset evidence.
- `app/src/live/` and `app/resources/views/live/` - persistent framework dogfood components, providers, and views.
- `manual/live.md` - application-facing authoring, routing, assets, deployment, testing, and recovery manual.
- `crates/suprnova-live/docs/implementation/framework-integration.md` - public ownership, topology, route, provider, and failure contracts.
- `crates/suprnova-live/docs/implementation/iteration-005-ledger.md` - exact task checkpoints and verification evidence.

## Task 1: Establish dependency direction and the curated public facade

**Files:**

- Modify: `framework/Cargo.toml`
- Modify: `framework/src/lib.rs`
- Create: `framework/src/live/mod.rs`
- Create: `framework/src/live/config.rs`
- Create: `framework/src/live/registry.rs`
- Create: `framework/src/live/testing.rs`
- Create: `framework/src/live/__private.rs`
- Create: `framework/src/view/mod.rs`
- Create: `framework/tests/live_public_api.rs`
- Create: `framework/tests/live_facade_contract.rs`
- Create: `framework/tests/live_dependency_topology.rs`
- Create: `framework/tests/fixtures/live-facade/Cargo.toml`
- Create: `framework/tests/fixtures/live-facade/src/lib.rs`

- [ ] **Step 1: Write failing facade and topology contracts**

Add an external workspace fixture whose manifest depends only on `suprnova` and compiles documented imports from `suprnova::live` and `suprnova::view`. Assert the framework manifest has one internal path dependency, assert the engine manifest does not depend on `suprnova`, and scan public examples for forbidden `suprnova_live` paths. Exercise public configuration, component registration, typed outcomes, checked HTML, and testing-helper names without reaching into `__private`. Build framework rustdoc without dependencies and scan the rendered public `live`/`view` signatures for `suprnova_live`, `suprnova_live_macros`, `askama_parser`, or another internal path outside the explicit hidden ABI; source-only string checks are insufficient.

Run:

```bash
rtk cargo test -p suprnova --test live_facade_contract --test live_dependency_topology
rtk cargo check --manifest-path framework/tests/fixtures/live-facade/Cargo.toml
```

Expected: failure because the real public modules and dependency do not exist.

- [ ] **Step 2: Add the internal dependency and minimal documented modules**

Add the non-published path dependency from `framework` to `../crates/suprnova-live`. Add `pub mod live` and `pub mod view` to the framework. Curate only application-facing types and wrappers; do not use `pub use suprnova_live::*`.

The first public surface must include:

- validated `LiveConfig` and builder;
- immutable `LiveRegistry` and builder accepting macro-produced component contracts;
- `ActionOutcome`, `ActionResult`, validation/error and testing contracts applications author against;
- `TrustedHtml` and checked document/view contracts under `suprnova::view`;
- production macros re-exported from the facade after Task 2;
- doc-hidden `live::__private` modules limited to the exact generated-code ABI.

- [ ] **Step 3: Prove public API and dependency direction**

Run:

```bash
rtk cargo test -p suprnova --test live_facade_contract --test live_dependency_topology
rtk cargo check --manifest-path framework/tests/fixtures/live-facade/Cargo.toml
rtk cargo check -p suprnova
rtk cargo doc -p suprnova --no-deps
rtk cargo test -p suprnova --test live_public_api
```

Expected: all pass with no undocumented public item and no engine-to-framework cycle.

- [ ] **Step 4: Review and checkpoint**

Run GitNexus impact for every touched public symbol, request an independent spec-compliance review, then an independent code-quality review. Apply findings, rerun the Task 1 commands, and commit only the verified Task 1 files.

## Task 2: Move production Live macros into `suprnova-macros`

**Files:**

- Modify: `suprnova-macros/src/lib.rs`
- Modify: `suprnova-macros/Cargo.toml`
- Create: `suprnova-macros/src/live/mod.rs`
- Create: `suprnova-macros/src/live/attrs.rs`
- Create: `suprnova-macros/src/live/component.rs`
- Create: `suprnova-macros/src/live/expand.rs`
- Create: `suprnova-macros/src/live/live_impl.rs`
- Modify: `framework/src/live/mod.rs`
- Modify: `framework/src/live/__private.rs`
- Move: `crates/suprnova-live/crates/suprnova-live-macros/tests/ui/**` to `suprnova-macros/tests/ui/live/**`
- Create: `suprnova-macros/tests/live_ui.rs`
- Create: `framework/tests/live_macro_expansion.rs`
- Remove after parity: `crates/suprnova-live/crates/suprnova-live-macros/**`
- Modify: `crates/suprnova-live/crates/suprnova-live-macro-fixture/Cargo.toml`
- Modify: `crates/suprnova-live/crates/suprnova-live-macro-fixture/src/lib.rs`
- Modify: `crates/suprnova-live/tests/fixtures/compile/{1,10,100}-component/Cargo.toml`
- Modify: `crates/suprnova-live/tests/fixtures/compile/{1,10,100}-component/src/lib.rs`
- Modify: `crates/suprnova-live/tests/{integrated_workspace.rs,workspace_contract.rs,license_inventory_graph.mjs,gate_contract.sh}`
- Modify: `crates/suprnova-live/scripts/{gate.sh,generate-license-inventory.mjs}`
- Modify: root `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Port the UI suite before implementation ownership changes**

Add `trybuild` to `suprnova-macros` dev-dependencies. Wire the existing pass/fail fixtures into the production macro crate and make them compile through the real `suprnova::live` facade. Add source-token assertions that expansions contain only `::suprnova::live` and `::suprnova::live::__private`, never an internal crate or relative runtime path.

Run:

```bash
rtk cargo test -p suprnova-macros --test live_ui
rtk cargo test -p suprnova --test live_macro_expansion
```

Expected: failure because production macro entry points are not present.

- [ ] **Step 2: Port the proven implementation without changing diagnostics**

Move the implementation modules into `suprnova-macros`, expose `#[derive(LiveComponent)]` and `#[live]`, and re-export them through `suprnova::live` and the crate root where the framework's macro convention requires it. Preserve all stderr fixtures unless a real path/name change makes a focused update necessary.

- [ ] **Step 3: Retire duplicate production ownership**

Remove `suprnova-live-macros` from root workspace membership and delete its package after parity is proven. Repoint the macro facade fixture and the 1/10/100-component compile fixtures to production `suprnova-macros`. Update workspace topology assertions, license graph/generator, Live gate, gate-contract snapshots, package lists, and lockfile in the same checkpoint. The facade fixture remains engine-only ABI evidence but consumes the one production macro implementation; there is no retained second macro implementation.

- [ ] **Step 4: Verify macro parity and hygiene**

Run:

```bash
rtk cargo test -p suprnova-macros --test live_ui
rtk cargo test -p suprnova --test live_macro_expansion
rtk cargo test -p suprnova --test macro_hygiene_qualified_paths
rtk cargo test -p suprnova-live --test metadata_contract --test binding_metadata --test component_support
rtk cargo clippy -p suprnova-macros --all-targets
```

Expected: exact diagnostics and expansion contracts pass without a duplicated production macro package.

- [ ] **Step 5: Review and checkpoint**

Run GitNexus impact, independent spec review, and independent quality review. Apply findings, rerun Task 2, and commit only verified macro/facade changes.

## Task 3: Build the downstream-only checked view authoring boundary

**Files:**

- Modify: `framework/Cargo.toml`
- Modify: `framework/src/view/mod.rs`
- Create: `framework/src/view/response.rs`
- Create: `suprnova-macros/src/view.rs`
- Modify: `suprnova-macros/src/lib.rs`
- Modify: `framework/src/live/__private.rs`
- Create: `framework/tests/live_view_contract.rs`
- Create: `framework/tests/templates/live/*.html`
- Create: `framework/tests/fixtures/live-authoring/Cargo.toml`
- Create: `framework/tests/fixtures/live-authoring/src/lib.rs`
- Create: `framework/tests/live_external_authoring.rs`

- [ ] **Step 1: Write failing checked-render and document-route tests**

Cover Askama-backed rendering through `suprnova::view`, checked unescaped `TrustedHtml`, ordinary escaped values, and typed status/headers/cache intent. The external fixture has its own workspace boundary and exactly one application dependency, `suprnova`; it uses the Suprnova-owned view attribute, implements a component view, and compiles without naming Askama or the internal engine in its manifest or source.

Run:

```bash
rtk cargo test -p suprnova --test live_view_contract --test live_external_authoring
```

Expected: failure because framework view/document adapters do not exist.

- [ ] **Step 2: Implement the public checked view wrapper**

Add the exact Askama dependency to `framework` as an implementation dependency. Implement Suprnova-owned `#[suprnova::view(path = "...")]` and `#[suprnova::view_filter]` attributes in `suprnova-macros`: they place generated Askama work in unique hidden modules, apply Askama's supported runtime override as `#[template(askama = ::suprnova::live::__private::askama, path = "...")]`, apply the derive/filter contracts there, and re-export the original application items at their declared visibility. The pinned Askama 0.16 source defines the override key as `askama`, not `crate`; do not substitute an unsupported attribute. Generated references use only the hidden Live ABI; application source and manifests never name Askama. The fixture must cover multiple templates in one module, generics, outer-module field types, visibility, custom filters, and compile diagnostics so the wrapper is not a happy-path illusion. Keep parser and engine internals hidden while exposing Suprnova-owned rendering traits, errors, and response helpers.

- [ ] **Step 3: Prove the external authoring ABI before route work**

Run the external fixture using a fresh target directory and inspect its manifest/source to prove `suprnova` is its only framework/view dependency. Add compile-fail cases for direct internal engine/parser paths and for bypassing checked `TrustedHtml` construction.

- [ ] **Step 4: Verify and review**

Run Task 3 tests, the external fixture's `cargo check`, relevant engine view tests, Clippy for `suprnova` and `suprnova-macros`, then independent spec and quality reviews. Commit after all findings and commands pass.

## Task 4: Bootstrap the Live runtime and trusted request-context adapter

**Files:**

- Create: `framework/src/live/runtime.rs`
- Create: `framework/src/live/context.rs`
- Create: `framework/src/live/attestation.rs`
- Create: `framework/src/live/tenant.rs`
- Create: `framework/src/live/ports/mod.rs`
- Create: `framework/src/live/ports/authorization.rs`
- Create: `framework/src/live/ports/transaction.rs`
- Create: `framework/src/live/ports/validation.rs`
- Create: `framework/src/live/ports/events.rs`
- Create: `framework/src/live/ports/telemetry.rs`
- Modify: `framework/src/live/config.rs`
- Modify: `framework/src/live/registry.rs`
- Modify: `framework/src/app/mod.rs`
- Modify: `framework/src/server.rs`
- Modify: `framework/src/http/request.rs`
- Modify: `framework/src/session/middleware.rs`
- Modify: `framework/src/csrf/middleware.rs`
- Modify: `framework/src/auth/middleware.rs`
- Modify: `framework/src/rate_limit/mod.rs`
- Modify: `framework/src/rate_limit/throttle.rs`
- Create: `framework/tests/live_boot.rs`
- Create: `framework/tests/live_trusted_context.rs`

- [ ] **Step 1: Write failing boot and hostile-context tests**

Prove deterministic boot, immutable registration after boot, key/config validation, route/slot catalog ownership, bounded context lifetime, body-cap ownership, session/principal/tenant fingerprints, current CSRF/origin/auth/authorization/rate dispositions, proxy normalization, capability-scope binding, missing-provider failures, cancellation, and safe redacted diagnostics. Add branch-specific tests showing that middleware omission, wrong order, exception/bypass branches, short-circuit responses, and stale request reuse cannot mint acceptable proof for action, upload, SSE-control, or WebSocket-handshake admission.

Run:

```bash
rtk cargo test -p suprnova --test live_boot --test live_trusted_context
```

Expected: failure because no runtime graph or context adapter exists.

- [ ] **Step 2: Make application startup fallible and order it before route construction**

Add an error-returning route/startup path while retaining the existing infallible `Application::routes` compatibility wrapper. Refactor server startup so service registration and boot complete first, then `LiveRuntime` is validated/bound, then the fallible route closure installs routes, then `Server` begins listening. Give programmatic `Server::new`/`Server::from_config` users the same prepared-runtime path; no test-only lifecycle may make Live work when the public server cannot. Runtime or route-construction errors return from the fallible boundary and are rendered once by the existing top-level boot error policy rather than panicking or partially listening.

- [ ] **Step 3: Build one immutable runtime graph**

Construct and validate `LiveRuntime` during the new ordered startup from `LiveConfig`, `LiveRegistry`, clock/random/key ring, ledger, execution kernel, and configured host ports. Register the runtime in Suprnova's container with an explicit override seam for tests. Reject invalid or colliding configuration before route construction or traffic.

- [ ] **Step 4: Mint non-forgeable security evidence at its owning middleware**

Add a crate-private request-carried `LiveSecurityAttestation` whose constructors and mutation methods are visible only to framework middleware. Session, CSRF/origin, authentication, and both rate-limit middleware families mint their own bounded proof only after the exact successful branch and bind it to request identity, route policy, and current scope. A configured `LiveTenantResolver` plus route middleware resolves the tenant and mints tenant proof; absence is explicit when the mount contract permits it. Applications can read ordinary framework state but cannot construct, clone across requests, or mark these proofs passed. WebSocket proof is finalized before upgrade.

- [ ] **Step 5: Normalize current framework facts once per request**

Create `LiveRequestContextCandidate` only after ordinary Suprnova middleware has produced current facts. Derive purpose-separated fingerprints without retaining raw credentials. Bind action/upload/subscription capabilities to the exact scope and run the engine validator before endpoint work. Never infer a passed security check from mere header presence.

- [ ] **Step 6: Implement host-owned application ports**

Adapt Suprnova authorization, transaction, validation, event/outbox/broadcast, telemetry, cancellation, and response-intent facilities to the engine traits. Every file under `live/ports` contains only `pub(crate)` concrete adapters implementing engine-owned traits: it introduces no parallel public port trait, outcome enum, retry policy, revision transition, or state table. Add topology/source assertions for that rule. State per-tier accepted-outcome behavior explicitly; action bodies remain safe to invoke again before commit and external side effects are not exactly-once.

- [ ] **Step 7: Verify and review**

Run Task 4 tests plus engine trusted-context/security/execution tests and focused framework session/auth/CSRF/origin tests. Run independent spec and quality reviews, apply findings, and commit.

## Task 5: Register the real action and fresh-render HTTP endpoints

**Files:**

- Create: `framework/src/live/routes.rs`
- Create: `framework/src/live/response.rs`
- Create: `framework/src/live/action.rs`
- Create: `framework/src/live/document.rs`
- Modify: `framework/src/live/runtime.rs`
- Modify: `framework/src/live/registry.rs`
- Modify: `framework/src/live/mod.rs`
- Modify: `framework/src/routing/router.rs`
- Create: `framework/tests/live_routes.rs`
- Create: `framework/tests/live_hostile_adapter.rs`
- Create: `framework/tests/live_document_routes.rs`
- Create: `framework/tests/fixtures/live-server/Cargo.toml`
- Create: `framework/tests/fixtures/live-server/src/main.rs`

- [ ] **Step 1: Write failing route and response-translation tests**

Cover strict single-shot Live installation; literal, wildcard, and dynamic overlap; exact POST/media/charset/cache rules; bounded buffered and streaming bodies; current trusted context; canonical no-store/security headers; typed status/recovery mapping; correlation; malformed/oversized/stale/cross-session/cross-tenant/cross-origin/unauthorized/duplicate/retired cases; and proof that rejected traffic performs no component/application work. Cover canonical Askama-backed documents, public seed and identity-bound mount emission after current scope resolution, duplicate island identities, unsupported contracts, no-JavaScript initial visibility, and whole-document failure before partial bytes.

Run:

```bash
rtk cargo test -p suprnova --test live_routes --test live_hostile_adapter
```

Expected: failure because reserved routes and adapters do not exist.

- [ ] **Step 2: Install versioned routes through the existing Router**

Expose one documented registration API that adds document support plus the versioned Live endpoint; protocol-v2 `fresh_render` remains an operation on that endpoint and does not create a second state machine. Add a private versioned Live-installation marker and atomic namespace preflight to `Router`. Installation is strictly single-shot: every second installation is an error, including an identical one. Preflight detects literal, parameterized, and wildcard overlap before mutating the router, then uses the existing route middleware and body collection APIs.

- [ ] **Step 3: Translate typed requests and responses without a second state machine**

Build engine request types after framework checks, invoke `LiveEndpointService` with the protocol operation (including v2 `fresh_render`), then convert complete typed HTTP intent into `HttpResponse`, preserving all security/cache/content headers and exact bytes. Framework code does not parse, mutate, or recreate protocol outcomes already owned by the engine.

- [ ] **Step 4: Integrate documents only after runtime/context exists**

Bind each registered route/slot to an immutable component/view contract during the ordered Task 4 startup. Produce public seed snapshots only for public mounts and identity-bound instance snapshots only after current request scope is resolved. Emit typed runtime-bootstrap requirements with the document and fail the whole document before partial bytes for duplicate/colliding/invalid mounts.

- [ ] **Step 5: Prove the minimal real-server path immediately**

Compile and start the downstream fixture whose manifest depends only on `suprnova`, request one document, execute one Live update through real middleware and routes, and shut it down cleanly. This is the early route acceptance test; Task 10 remains the full persistent dogfood workflow.

- [ ] **Step 6: Verify rejection ordering and review**

Run Task 5 tests, engine endpoint/failure/order suites, focused CSRF/origin/body-cap tests, and Clippy. Request independent spec and quality reviews, apply findings, and commit.

## Task 6: Adapt upload control, data, and application reacquisition

**Files:**

- Create: `framework/src/live/upload.rs`
- Create: `framework/src/live/ports/upload.rs`
- Create: `framework/src/live/ports/upload_ledger.rs`
- Create: `framework/src/live/ports/upload_provider.rs`
- Create: `framework/src/live/ports/upload_validation.rs`
- Create: `framework/src/live/ports/upload_finalizer.rs`
- Modify: `framework/src/live/routes.rs`
- Modify: `framework/src/live/runtime.rs`
- Create: `framework/tests/live_upload_routes.rs`
- Create: `framework/tests/live_upload_security.rs`
- Create: `framework/tests/live_upload_providers.rs`

- [ ] **Step 1: Write failing upload boundary tests**

Cover control/data separation, declared size/type/count, actual media validation, quarantine before finalization, current authorization, ownership/scope binding, transfer tokens, replay/expiry/range/offset failures, direct-provider grants, abort/cleanup/cancellation, scanner/storage failures, and final action proposal of only a ready signed handle. Run the engine provider conformance cases through framework implementations of the upload ledger, quarantine store, cleanup ownership, byte provider, authorization, scanner, application validator, validation-evidence store, and finalizer; test each missing/default/failing provider independently.

Run:

```bash
rtk cargo test -p suprnova --test live_upload_routes --test live_upload_security --test live_upload_providers
```

Expected: failure because framework upload adapters and routes do not exist.

- [ ] **Step 2: Adapt Suprnova storage and validation to engine upload ports**

Keep host-owned I/O in separate `pub(crate)` framework adapters, bounded and cancellation-aware. Map every distinct engine port explicitly: ledger persistence, quarantine byte storage, cleanup, provider reads/writes/direct grants, current authorization, scanner, application validation, immutable validation evidence, and finalization. The engine remains authority for upload state, identity, grants, quarantine transitions, and finalize semantics. No framework adapter combines those transition tables or lets a handler receive an unready path or raw client file authority.

- [ ] **Step 3: Register reserved upload routes and explicit reacquisition**

Install reserved control/data routes under the versioned Live namespace. Provide a documented handler helper for an authenticated application route outside `/__live/` to reacquire resumable authority; never silently register an application-owned URL. Add that authenticated route to the persistent root dogfood application and exercise lost-token, cross-session, cross-tenant, expired, and successful resume end-to-end through current middleware.

- [ ] **Step 4: Verify and review**

Run Task 6 tests and the full engine upload suite, then independent spec/security-quality reviews. Apply findings and commit.

## Task 7: Adapt polling, SSE, and WebSocket augmentation

**Files:**

- Create: `framework/src/live/async_updates.rs`
- Create: `framework/src/live/ports/subscription.rs`
- Modify: `framework/src/live/routes.rs`
- Modify: `framework/src/live/runtime.rs`
- Create: `framework/tests/live_async_routes.rs`
- Create: `framework/tests/live_async_security.rs`
- Create: `framework/tests/live_async_backpressure.rs`

- [ ] **Step 1: Write failing asynchronous transport tests**

Cover poll through ordinary fresh-render scheduling, one multiplexed document stream, typed events/invalidations only, descriptor credentials, current registry/topic authorization, continuity/replay windows, SSE and WebSocket origin rules, cross-site WebSocket hijacking, reconnect, cancellation, backpressure, bounded fan-in, and one chatty island not starving another. Prove the socket loop never reads session/auth/container/request-id or other ambient task-local request state after upgrade.

Run:

```bash
rtk cargo test -p suprnova --test live_async_routes --test live_async_security --test live_async_backpressure
```

Expected: failure because production async route adapters do not exist.

- [ ] **Step 2: Bind framework broadcast and transport facilities**

Adapt current authorization, credential issuance, registry/topic resolution, continuity, and broadcasting to engine subscription ports. SSE uses `HttpResponse::event_stream`; WebSocket uses existing per-route middleware and origin policy. Stream messages enter the normal island scheduler as typed events or invalidations and never carry trusted HTML or arbitrary actions.

During the middleware-covered WebSocket handshake, normalize and validate one bounded immutable Live context plus only the owned transport facts the socket task needs. Move that value into the upgraded task; do not move raw credentials or depend on ambient request task-locals. Reauthorize every subsequent control, renewal, reconnect, and topic-change operation against current host authority rather than treating the handshake snapshot as perpetual authorization.

- [ ] **Step 3: Register versioned control/stream routes**

Install HTTP control plus SSE and WebSocket transport paths with exact media, cache, origin, rate, cancellation, and telemetry behavior. Preserve the engine's multiplexing and bounded-resource contracts instead of creating one connection per island.

- [ ] **Step 4: Verify and review**

Run Task 7 tests, engine async suites, framework SSE/WS/origin tests, and Clippy. Request independent spec and quality reviews, apply findings, and commit.

## Task 8: Serve exact deterministic runtime artifacts through Suprnova

**Files:**

- Create: `framework/src/live/assets.rs`
- Modify: `framework/src/live/document.rs`
- Modify: `framework/src/live/routes.rs`
- Modify: `framework/build.rs` if compile-time embedding requires it
- Create: `framework/tests/live_assets.rs`
- Create: `framework/tests/live_asset_browser.rs`
- Create: `crates/suprnova-live/browser/e2e/framework-bootstrap.spec.ts`
- Modify: `crates/suprnova-live/browser/scripts/**` only if a verified manifest-consumer path is missing

- [ ] **Step 1: Write failing manifest and real-route tests**

For every required ESM/classic core and optional Stimulus/upload/async artifact, assert path, media type, byte length, digest/integrity, capability/version metadata, ETag/conditional GET/HEAD, immutable cache policy, CSP-safe delivery, deduplication, and exact equality with the reviewed browser output. Add a browser scenario against the real downstream Suprnova server that exercises ESM and classic role selection, optional Stimulus/upload/async roles, duplicate bootstrap tags/islands, missing/incompatible roles, integrity failure, strict CSP, and preservation of visible SSR content.

Run:

```bash
rtk cargo test -p suprnova --test live_assets
```

Expected: failure because the framework does not serve Live artifacts.

- [ ] **Step 2: Embed or include exact reviewed bytes**

Consume the typed production manifest and artifact bytes without rebuilding at framework runtime or compile time. Fail the build/test if manifest digest, length, filename, or capability metadata disagrees. Keep Stimulus optional and do not introduce a package-manager requirement for application users.

- [ ] **Step 3: Emit typed bootstrap requirements from documents**

Have the document response map registered component capabilities to a bounded typed artifact-role set and emit one deterministic ESM or classic bootstrap strategy with integrity/CSP metadata. Optional roles load only when required; repeated islands and nested documents do not duplicate a role. Missing/incompatible required roles fail document construction before partial bytes rather than silently degrading interaction.

- [ ] **Step 4: Register immutable asset routes**

Serve GET and HEAD through Suprnova's existing response/static facilities with correct validators, cache headers, MIME, length, CSP/integrity metadata, and traversal-resistant fixed lookup. Unknown or retired artifact names return a closed miss.

- [ ] **Step 5: Verify and review**

Run Task 8 tests, the real-framework Playwright scenario, browser build/budget/artifact checks, CSP tests, then independent spec and quality reviews. Apply findings and commit.

## Task 9: Add non-destructive Live CLI workflows

**Files:**

- Modify: `suprnova-cli/Cargo.toml`
- Modify: `suprnova-cli/src/main.rs`
- Modify: `suprnova-cli/src/commands/mod.rs`
- Create: `suprnova-cli/src/commands/live_make.rs`
- Create: `suprnova-cli/src/commands/live_check.rs`
- Create: `suprnova-cli/src/commands/live_inspect.rs`
- Create: `suprnova-cli/src/commands/live_assets.rs`
- Create: `suprnova-cli/src/templates/files/backend/live/component.rs.tpl`
- Create: `suprnova-cli/src/templates/files/backend/live/view.html.tpl`
- Create: `suprnova-cli/src/templates/files/backend/live/mod.rs.tpl`
- Modify: backend scaffold templates only where registration is required
- Create: `suprnova-cli/tests/live_cli.rs`
- Create: `suprnova-cli/tests/live_scaffold.rs`
- Create: `suprnova-cli/tests/live_assets.rs`
- Create: `framework/src/live/tooling.rs`
- Create: `framework/src/live/tooling_protocol.rs`
- Modify: `suprnova-cli/src/templates/files/backend/src/bin/console.rs.tpl`
- Create: `framework/tests/live_tooling_protocol.rs`

- [ ] **Step 1: Write failing CLI contract tests**

Cover `live:make`, `live:check`, `live:inspect`, and `live:assets` help/argument behavior; dry-run; invalid names; existing-file conflicts; atomic writes; path traversal; symlink refusal; checker diagnostics; redacted inspection; deterministic asset publication; and repeat-run idempotence. Cover missing child helper, nonzero exit, unsupported protocol, stale build identity, malformed/truncated/oversized JSON line, excessive diagnostics/assets, digest mismatch, unexpected stdout, and secret/body redaction.

Run:

```bash
rtk cargo test -p suprnova-cli --test live_cli --test live_scaffold --test live_assets
```

Expected: failure because Live commands and templates do not exist.

- [ ] **Step 2: Implement one versioned application-tooling subprocess protocol**

Keep `suprnova-cli` free of framework and internal-engine runtime dependencies. The generated console binary exposes a doc-hidden application helper invoked as `__suprnova:live-tool --protocol 1 --operation <check|inspect|assets>`. The CLI starts it through the existing explicit-binary Cargo wrapper and consumes bounded JSON-lines v1 from stdout; human/build output remains on stderr. Every envelope carries protocol, framework/build identity, operation, success/failure, redacted payload, and an end marker. Set explicit per-line, total-byte, diagnostic-count, asset-count, and asset-byte caps.

The in-application helper owns registry access, checked-template/contract validation, safe inspection, production-manifest compatibility, and asset-export assembly. `assets` returns already validated fixed-name bytes plus length/digest in the bounded envelope; the CLI verifies transport length/digest and writes them but does not implement a second compatibility/manifest parser. Unsupported, stale, corrupt, missing, truncated, or oversized helper output fails closed with no writes and no fallback checker.

- [ ] **Step 3: Implement commands using existing secure filesystem conventions**

Scaffold semantic Rust/view/registration files without overwriting user work. `live:check`, `live:inspect`, and `live:assets` are thin protocol clients for the application helper. Inspection reports only the helper's safe bounded metadata and provider/config state. Asset publication is atomic and refuses drift unless the user explicitly selects the existing safe replacement mode.

- [ ] **Step 4: Verify and review**

Run Task 9 tests plus existing CLI scaffold/template-drift tests, then independent spec and quality reviews. Apply findings and commit.

## Task 10: Prove a freshly generated application and real framework paths

**Files:**

- Create: `app/src/live/mod.rs`
- Create: `app/src/live/components/*.rs`
- Create: `app/src/live/providers/*.rs`
- Create: `app/resources/views/live/*.html`
- Modify: `app/src/lib.rs`
- Modify: `app/src/bootstrap.rs`
- Modify: `app/src/routes.rs`
- Create: `app/tests/live_dogfood.rs`
- Create: `app/tests/live_upload_reacquire.rs`
- Create: `app/tests/live_async_dogfood.rs`
- Create: `framework/tests/live_dogfood.rs`
- Create: `suprnova-cli/tests/live_generated_app.rs`
- Create: `manual/live.md`
- Modify: `manual/README.md`
- Modify: `crates/suprnova-live/docs/implementation/framework-integration.md`
- Modify: `crates/suprnova-live/docs/implementation/iteration-005-ledger.md`

- [ ] **Step 1: Write the generated-application acceptance harness**

Generate a fresh backend application in a temporary directory, scaffold a Live component, register routes/providers, compile and run its minimal path, and assert its Cargo manifest depends only on `suprnova` for Live use. Separately, install the complete durable dogfood surface in the repository's existing root `app/`; a disposable fixture cannot satisfy this acceptance requirement.

Run:

```bash
rtk cargo test -p suprnova-cli --test live_generated_app
rtk cargo test -p suprnova --test live_dogfood
rtk cargo test -p app --test live_dogfood --test live_upload_reacquire --test live_async_dogfood
```

Expected: failure until the complete public path is wired.

- [ ] **Step 2: Build the persistent root-application dogfood surface**

Add ordinary SSR and Live routes, semantic components/views, explicit providers, an authenticated upload-reacquisition route outside `/__live/`, polling, SSE and WebSocket augmentation, forced stale/morph/transport recovery, and no-build production asset delivery to `app/`. Exercise those through real application tests and a browser scenario. Fix the public integration rather than special-casing either fixture. Keep RenderCache disabled here; Live-with-RenderCache belongs to the separate RenderCache plan.

- [ ] **Step 3: Document the public framework contract**

Add `manual/live.md` to the public manual index. Document authoring, registration, boot/config/providers, route ownership, ordinary SSR, Live without RenderCache, uploads/reacquisition, polling/SSE/WebSocket, asset bootstrap/no-build use, testing, diagnostics, operations, security boundaries, and recovery. Examples use no internal package names. Record exact commits and commands in the Iteration 005 ledger.

- [ ] **Step 4: Verify and review**

Run the generated-app and dogfood tests, the integrated Live gate, affected Suprnova tests, independent spec review, and independent quality review. Apply findings and commit.

## Task 11: Run the framework-integration qualification gate

**Files:**

- Modify only if evidence exposes a real defect: affected implementation/spec/test files
- Modify: `crates/suprnova-live/docs/implementation/iteration-005-ledger.md`

- [ ] **Step 1: Run focused static and test gates**

```bash
rtk cargo fmt --all --check
rtk cargo clippy -p suprnova-live -p suprnova-macros -p suprnova -p suprnova-cli --all-targets --all-features
rtk cargo test -p suprnova-live --all-targets --all-features --no-fail-fast
rtk cargo test -p suprnova-live --doc --all-features
rtk cargo test -p suprnova-macros --all-targets
rtk cargo test -p suprnova --all-targets --all-features --no-fail-fast
rtk cargo test -p suprnova --doc --all-features
rtk cargo test -p suprnova-cli --all-targets --no-fail-fast
rtk cargo metadata --format-version 1 --no-deps
rtk cargo check --workspace --all-targets --all-features
rtk cargo test -p suprnova-live-macro-fixture -p suprnova-live-test-support --all-targets --all-features
```

Expected: all pass without blanket `-D warnings`.

- [ ] **Step 2: Run the integrated browser and Live gates**

```bash
rtk bash -lc 'cd crates/suprnova-live/browser && npm ci && npm run format:check && npm run lint && npm run typecheck && npm test && npm run build && npm run budget'
rtk node crates/suprnova-live/scripts/check-specs.mjs
rtk node crates/suprnova-live/scripts/check-implementation-docs.mjs
rtk /home/shawn/workspace2/suprnova/scripts/check-suprnova-live.sh
rtk /home/shawn/workspace2/suprnova/scripts/gate.sh
```

Expected: all ordinary integrated checks pass. Existing Iteration 004 pinned-environment qualification remains reported honestly and is not relabeled by this plan.

- [ ] **Step 3: Audit architecture and drift**

Run codebase-memory `detect_changes` against the task baseline, GitNexus change/impact detection, `rtk tilth diff`, `rtk git diff --check`, and the Same Page drift gate. Confirm:

- no engine-to-framework dependency;
- no application-facing internal crate path;
- no duplicate production macro or route state machine;
- no retired `suprnova-live-macros` package/member/reference and retained fixture/support packages still pass;
- no RenderCache implementation;
- no Suprnova Magnetar file modified by this plan;
- all new vocabulary and changed truth are reflected in glossary/spec/implementation docs.

- [ ] **Step 4: Adversarial review and checkpoint**

Request one independent adversarial architecture/security review of the complete framework integration and one independent production-quality review. Resolve every blocker, rerun every affected command and the complete Task 11 gate, update the ledger with exact evidence, and commit locally. Do not push.

## Completion handoff

Framework integration is complete only when Iteration 005 definition-of-done items 4 through 9 are demonstrated through real Suprnova APIs and paths, the ordinary integrated gate passes, and outstanding Iteration 004 pinned-environment qualification remains visible. The next work item is a separately written and adversarially reviewed RenderCache implementation plan covering Iteration 005 definition-of-done items 10 through 29.
