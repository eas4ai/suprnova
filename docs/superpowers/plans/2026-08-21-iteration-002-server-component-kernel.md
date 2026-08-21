# Suprnova Live Iteration 002 Server-Component Kernel Implementation Plan

> **Execution rule:** Implement this plan inline, task by task, with
> `superpowers:test-driven-development` and
> `superpowers:verification-before-completion`. Do not delegate unless the user
> explicitly requests sub-agents. Keep exactly one task in progress, and commit
> only after its implementation and listed verification pass.

**Goal:** Turn the Iteration 001 trusted interaction spine into the complete
standalone, host-neutral Suprnova Live server-component kernel: checked Askama
views, generated component metadata, explicit registration, typed state and
binding, lifecycle and composition, actions and validation, signed child
authority, protocol v2, trusted host context, endpoint response intent,
conformance tooling, and the A8/16 action-framework budget.

**Architecture:** The existing `suprnova-live` engine remains the only runtime
authority. An internal proc-macro development crate emits final
`::suprnova::live` paths and static metadata; a dev-only facade fixture proves
those paths before the later atomic move. An explicitly built immutable registry
resolves browser-visible identities to closed descriptors. Views render through
Askama into separate document or island result types. Every request reconstructs
a component from a verified capability, applies bounded typed proposals, invokes
only registered lifecycle/action hooks, renders and signs a complete successor,
then coordinates host transaction and ledger acceptance in the locked order.
No sticky component object, hidden response store, Suprnova checkout dependency,
or browser runtime is introduced.

**Pinned additions:** Askama and `askama_parser` 0.16.0; `html5ever` 0.39.0;
`bytes` 1.11.1; `http` 1.4.1; `time` 0.3.47; `uuid` 1.23.1;
`proc-macro2` 1.0.106; `quote` 1.0.45; `syn` 2.0.117; `trybuild` 1.0.120.
All remain exact lockfile inputs and must pass the repository license inventory.

## Locked implementation decisions

- Protocol v1 remains byte- and meaning-stable. Protocol v2 adds
  `params_changed`, `lazy_complete`, `fresh_render`, signed child delivery, and
  typed URL intent. Snapshot schema v1 is independently versioned.
- Default component `State` is instance-only. Only explicit `Public` fields may
  enter reusable public seeds. `Session`, `ServerOnly`, `Computed`, `Transient`,
  and `Secret` never dehydrate.
- Private initial mounts use atomic `mount_instance`; they are never disguised
  as seed promotion. Output is not publishable until ledger authority exists.
- Generated code names only `::suprnova::live` and
  `::suprnova::live::__private`. The standalone fixture is allowed to impersonate
  that facade only as a dev dependency.
- Components register explicitly into an immutable startup registry. There is no
  `inventory`, linker section, global mutable registry, or browser-selected Rust
  type path.
- Checked templates use `askama_parser` for template structure and `html5ever`
  for HTML structure. Untyped raw `safe` is rejected; trusted markup requires
  `TrustedHtml` plus an auditable reason.
- A versioned semantic idempotency digest excludes correlation and transport
  facts while binding all authority and requested work. Accepted ledger metadata
  does not retain response bodies; an unreplayable duplicate refreshes without
  action execution.
- Tier 0 ordering is claim -> hydrate/bind -> authorize/validate -> optional host
  transaction -> before/action/after -> render/dehydrate/sign/validate -> host
  commit -> ledger accept -> non-authoritative reporting. A durable host commit
  followed by failed ledger acceptance requires fresh render, never replay.
- Document renders may express document response intent. Island renders cannot
  inject status, headers, cookies, cache policy, or media type; the endpoint owns
  them.
- Conformance adapters prove the kernel contract only. Actual Suprnova router,
  request/response, session, CSRF, auth, tenant, validation, and transaction
  adapters wait for the atomic move.

## Task 1: Expand the reproducible workspace without changing the public facade

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `rust-toolchain.toml`
- Modify: `src/lib.rs`
- Modify: `tests/workspace_contract.rs`
- Create: `crates/suprnova-live-macros/Cargo.toml`
- Create: `crates/suprnova-live-macros/src/lib.rs`
- Create: `crates/suprnova-live-macro-fixture/Cargo.toml`
- Create: `crates/suprnova-live-macro-fixture/src/lib.rs`
- Create: `crates/suprnova-live-test-support/Cargo.toml`
- Create: `crates/suprnova-live-test-support/src/lib.rs`
- Modify: `scripts/generate-license-inventory.mjs`
- Modify: `THIRD_PARTY_LICENSES.md`

- [ ] First extend `tests/workspace_contract.rs` to require supported protocols
  `[1, 2]`, independently supported snapshot schema `[1]`, the three internal
  development packages, `publish = false` on helper packages, and absence of
  default feature drift. Run `rtk env CARGO_INCREMENTAL=0 cargo test --test
  workspace_contract`; observe the expected version/workspace failure.
- [ ] Add the workspace members and exact dependencies listed above. The engine
  may depend on Askama/parser/HTML/HTTP/value types; the macro crate depends only
  on proc-macro parsing/generation crates; the fixture depends on the engine;
  test support depends on the engine as a dev tool. Do not add a Suprnova or
  Magnetar source/path dependency.
- [ ] Export protocol support as `SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[1, 2]`
  while retaining `SUPPORTED_SNAPSHOT_VERSIONS: &[u16] = &[1]`. Add no fake
  application-facing `suprnova::live` module to the engine.
- [ ] Make the macro fixture expose the shape that final integration will own:

  ```rust
  pub mod live {
      pub use suprnova_live::*;

      #[doc(hidden)]
      pub mod __private {
          pub use suprnova_live::metadata::*;
      }
  }
  ```

  The fixture must not re-export procedural macros; macro UI cases import the
  development macro crate separately and resolve generated runtime paths through
  the fixture renamed to `suprnova`.
- [ ] Regenerate/check the exact license inventory and lockfile. Run
  `rtk env CARGO_INCREMENTAL=0 cargo check --workspace --all-targets
  --all-features` and the targeted test.
- [ ] Commit: `build: prepare iteration 002 kernel workspace`.

## Task 2: Add canonical component metadata and the explicit immutable registry

**Files:**

- Create: `src/metadata/mod.rs`
- Create: `src/metadata/component.rs`
- Create: `src/metadata/field.rs`
- Create: `src/metadata/method.rs`
- Create: `src/metadata/version.rs`
- Create: `src/metadata/digest.rs`
- Create: `src/registry/mod.rs`
- Create: `src/registry/builder.rs`
- Create: `src/registry/descriptor.rs`
- Create: `src/registry/error.rs`
- Modify: `src/lib.rs`
- Create: `tests/metadata_contract.rs`
- Create: `tests/registry_contract.rs`

- [ ] Write failing tests for independent nonzero component, state-schema,
  action, checker, and minimum-protocol versions; duplicate component/view/action
  names; conflicting field attributes; unstable metadata order; duplicate
  startup registration; and a browser string that does not resolve outside the
  registry. Run `rtk env CARGO_INCREMENTAL=0 cargo test --test metadata_contract
  --test registry_contract`; confirm missing-module failures.
- [ ] Implement closed metadata values and a canonical contract digest. The core
  shape is:

  ```rust
  pub struct ComponentMetadata {
      pub identity: ComponentName,
      pub view: ViewName,
      pub component_version: u16,
      pub state_schema_version: u16,
      pub action_schema_version: u16,
      pub checker_contract_version: u16,
      pub minimum_protocol_version: u16,
      pub fields: &'static [FieldMetadata],
      pub actions: &'static [ActionMetadata],
  }

  pub struct ComponentDescriptor {
      metadata: &'static ComponentMetadata,
      contract_digest: ContentDigest,
  }
  ```

  Digest a purpose/version prefix plus canonical semantic metadata. Rust type
  paths, source paths, addresses, and registration order are not digest input.
- [ ] Implement `ComponentRegistryBuilder::register(descriptor)` and
  `build() -> Result<ComponentRegistry, RegistryError>`. `ComponentRegistry` is
  immutable, `Send + Sync`, indexed by validated component identity, and exposes
  mount-catalog matching without any global or linker registration.
- [ ] Make all diagnostics closed and source-oriented. Runtime diagnostics may
  contain registered safe identities but never state, proposals, arguments,
  snapshots, or arbitrary browser strings.
- [ ] Run the targeted tests, `rtk env CARGO_INCREMENTAL=0 cargo test --doc
  --all-features`, and Clippy without `-D warnings`.
- [ ] Commit: `feat: add Live metadata and explicit registry`.

## Task 3: Generate the locked authoring contract through final facade paths

**Files:**

- Create: `crates/suprnova-live-macros/src/attrs.rs`
- Create: `crates/suprnova-live-macros/src/component.rs`
- Create: `crates/suprnova-live-macros/src/live_impl.rs`
- Create: `crates/suprnova-live-macros/src/expand.rs`
- Create: `crates/suprnova-live-macros/tests/ui.rs`
- Create: `crates/suprnova-live-macros/tests/ui/pass/minimal.rs`
- Create: `crates/suprnova-live-macros/tests/ui/pass/all_metadata.rs`
- Create: `crates/suprnova-live-macros/tests/ui/fail/*.rs`
- Create: `crates/suprnova-live-macros/tests/ui/fail/*.stderr`
- Modify: `crates/suprnova-live-macro-fixture/src/lib.rs`
- Modify: `tests/metadata_contract.rs`

- [ ] Add trybuild failures first for duplicate helpers, unknown helpers, invalid
  names/versions, unsupported generic or reference state, `#[model]` plus
  `#[locked]`, transient without model, session plus dehydrated exposure,
  sensitive/transient URL exposure, inaccessible component items, invalid action
  signatures, and helpers placed on the wrong item. Include
  `refresh_on_promote` with a minimum protocol below v2 as a compile failure. Run
  `rtk env CARGO_INCREMENTAL=0 cargo test -p
  suprnova-live-macros --test ui`; observe the expected compile failures before
  accepting `.stderr` files.
- [ ] Implement `#[derive(LiveComponent)]` with struct helper
  `#[live(name = "...", view = "...", events(...), effects(...), ...)]` and
  field helpers `#[public]`, `#[model]`, `#[model(transient)]`, `#[locked]`,
  `#[server_only]`, `#[session]`, and `#[secret]`. Unannotated fields emit
  `State` metadata. Event/effect paths must implement their versioned payload
  metadata traits; duplicate registered names fail deterministically.
- [ ] Implement outer `#[live]` on an inherent impl and consume method helpers
  `#[mount]`, `#[action]`, `#[computed]`, `#[validate]`, and the named lifecycle
  hook helpers. Unknown/misplaced helpers fail at their original `syn::Span`.
- [ ] Emit only absolute `::suprnova::live` and
  `::suprnova::live::__private` runtime references. Add a source-token assertion
  that rejects `suprnova_live`, the macro package name, `$crate`, or test-only
  module paths in expanded runtime code.
- [ ] Generate metadata in a stable field/method order and expose an explicit
  descriptor function for startup registration. Do not generate inventory
  submission or global mutation.
- [ ] Run trybuild, metadata/registry tests, `cargo expand` on the 1-component
  fixture, format, and Clippy review.
- [ ] Commit: `feat: generate Live component metadata`.

## Task 4: Complete state categories, typed codecs, and proposal application

**Files:**

- Modify: `src/snapshot/state.rs`
- Create: `src/state/mod.rs`
- Create: `src/state/codec.rs`
- Create: `src/state/proposal.rs`
- Create: `src/state/path.rs`
- Create: `src/state/session.rs`
- Create: `src/state/timing.rs`
- Create: `src/state/url.rs`
- Modify: `src/metadata/field.rs`
- Create: `tests/state_categories.rs`
- Create: `tests/model_binding.rs`
- Create: `tests/model_binding_properties.rs`
- Create: `tests/binding_metadata.rs`
- Modify: `tests/snapshot_state.rs`

- [ ] Write failing tests that prove `State` is instance-only, `Public` alone is
  seed-eligible, `Session` never dehydrates, and server-only/computed/secret/
  transient fields never cross snapshots or diagnostics. Add binding cases for
  missing, null, invalid, and valid scalar/option/list/map/nested/enum/date/
  datetime/UUID/checkbox/multi-select values and hostile unknown/conflicting/
  oversized/unstable paths.
- [ ] Extend `FieldCategory` to exactly:

  ```rust
  pub enum FieldCategory {
      State,
      Public,
      Model,
      Locked,
      ServerOnly,
      Session,
      Computed,
      Transient,
      Secret,
  }
  ```

  `StateExposure::Instanced` allows `State | Public | Model | Locked`; public
  seed exposure allows only `Public`. Required-field validation is
  exposure-aware: required public fields must exist in a seed, while intentionally
  absent instance-only and nondehydrated fields do not make that partial form
  invalid.
- [ ] Add registered typed codecs and the lossless proposal result:

  ```rust
  pub enum ProposedValue<T> {
      Missing,
      Null,
      Invalid(BindingIssue),
      Valid(T),
  }
  ```

  Reject an unknown/forbidden field or malformed path before calling any
  generated setter. Bound segments, nesting, collection entries, decoded bytes,
  issue count, and error text.
- [ ] Add host-neutral `SessionPort` reads/writes by registered field metadata.
  Session values hydrate from the current host context and leave only bounded
  session intent; they never enter dehydrated state/memo or diagnostics/logs.
  Any authorized derived presentation or emission remains explicit component
  output under ordinary escaping, schema, and authorization policy.
- [ ] Add property tests that successful typed decode/encode round-trips and any
  failed proposal leaves component state unchanged.
- [ ] Add closed `BindingTiming` metadata for immediate/change/blur/submit and
  bounded debounce declarations, plus `UrlBinding` metadata for same-route
  replace reflection or real-route navigation intent. Timing is checker/server
  contract only in this iteration; no timer or history API is implemented.
  Reject secret/transient/session/invalid URL state, duplicate query keys, and
  values that fail registered URL codecs.
- [ ] Run the targeted state/snapshot suite and Clippy review.
- [ ] Commit: `feat: add typed Live state and binding`.

## Task 5: Replace boolean promotion trust with typed host context

**Files:**

- Create: `src/host/mod.rs`
- Create: `src/host/context.rs`
- Create: `src/host/checks.rs`
- Create: `src/host/catalog.rs`
- Create: `src/host/capabilities.rs`
- Modify: `src/promotion/context.rs`
- Modify: `src/promotion/service.rs`
- Modify: `crates/suprnova-live-test-support/src/lib.rs`
- Create: `crates/suprnova-live-test-support/src/context.rs`
- Create: `tests/trusted_context.rs`
- Modify: `tests/promotion_support.rs`
- Modify: `tests/seed_promotion*.rs`

- [ ] Write compile/runtime failures proving production code has no public
  zero-input verified marker; missing checks, unchecked checks, expired facts,
  scope mismatch, route/slot mismatch, principal/tenant mismatch, and catalog
  mismatch cannot enter promotion or endpoint services.
- [ ] Introduce a closed disposition for every configured host authenticity
  check:

  ```rust
  pub enum CheckDisposition {
      Passed,
      NotRequired(PolicyReason),
  }

  pub struct TrustedLiveRequestContext {
      scope: ScopeFingerprint,
      mount: VerifiedMountCatalogMatch,
      checks: RequiredChecks,
      capabilities: HostCapabilities,
      expires_at: UnixMillis,
  }
  ```

  There is no `Unchecked`, boolean shortcut, raw cookie/header/token field, or
  reusable authorization decision.
- [ ] Make registry/catalog resolution bind route identity, slot identity,
  component identity, contract digest, principal/tenant scope requirements, and
  minimum protocol version. Browser input may select only an entry already
  matched by trusted host facts.
- [ ] Replace `TrustedPromotionContext` internals with a projection of
  `TrustedLiveRequestContext`; delete `PromotionAttestations::verified()`.
- [ ] Put ergonomic synthetic builders only in
  `suprnova-live-test-support`. They must call the complete production validator,
  not bypass it, and the engine must not expose a `test` feature.
- [ ] Run targeted promotion/security tests plus
  `rtk env CARGO_INCREMENTAL=0 cargo tree -e features` to prove test support is
  absent from production dependency edges.
- [ ] Commit: `feat: harden Live host request authority`.

## Task 6: Implement the Askama view contract and trusted markup boundary

**Files:**

- Create: `src/view/mod.rs`
- Create: `src/view/contract.rs`
- Create: `src/view/document.rs`
- Create: `src/view/island.rs`
- Create: `src/view/trusted_html.rs`
- Create: `src/view/error.rs`
- Create: `tests/view_contract.rs`
- Create: `tests/view_escaping.rs`
- Create: `tests/document_render.rs`
- Create: `tests/island_render.rs`
- Create: `templates/tests/*.html`

- [ ] Write failing Askama fixtures for default escaping, explicit trusted
  markup, render failure with no partial success, missing view data, multiple
  island roots, executable mount metadata, document-only headers/status, and an
  island attempting to inject status/header/cache/cookie intent.
- [ ] Implement distinct result types:

  ```rust
  pub struct DocumentRender {
      pub body: bytes::Bytes,
      pub response: DocumentResponseIntent,
      pub assets: AssetSet,
      pub mounts: Vec<MountMetadata>,
  }

  pub struct IslandRender {
      pub body: bytes::Bytes,
      pub assets: AssetSet,
      pub children: Vec<ChildMount>,
  }
  ```

  `IslandRender` has no status, header, cookie, cache, or media-type field.
- [ ] Add `TrustedHtml` with an explicit bounded reason and constructors for
  static framework markup or output carrying a registered sanitizer proof.
  Its only unescaped Askama integration is a Suprnova-owned checked filter. Raw
  Askama `safe` remains a checker error even when Rust types would compile.
- [ ] Buffer rendering to completion before constructing a successful result.
  Bound bytes/assets/mounts/diagnostics and return redacted source-oriented
  errors without partial output.
- [ ] Implement the canonical document conformance adapter for complete HTML,
  exposed initial content, HEAD body suppression, conditional response intent,
  and deterministic failures. Label it conformance, never a Suprnova route.
- [ ] Run all view tests, doctests, and Clippy review.
- [ ] Commit: `feat: add checked Askama view rendering`.

## Task 7: Reconstruct lifecycle and publish atomic private mounts

**Files:**

- Modify: `src/ledger/contract.rs`
- Modify: `src/ledger/memory.rs`
- Modify: `src/ledger/state.rs`
- Create: `src/component/mod.rs`
- Create: `src/component/instance.rs`
- Create: `src/component/lifecycle.rs`
- Create: `src/component/executor.rs`
- Modify: `src/registry/descriptor.rs`
- Modify: `crates/suprnova-live-macros/src/live_impl.rs`
- Create: `src/mount/mod.rs`
- Create: `src/mount/service.rs`
- Create: `src/mount/output.rs`
- Create: `src/mount/error.rs`
- Modify: `src/view/island.rs`
- Create: `tests/lifecycle_order.rs`
- Create: `tests/lifecycle_failures.rs`
- Create: `tests/ledger_mounts.rs`
- Create: `tests/initial_mount.rs`
- Create: `tests/initial_mount_failures.rs`

- [ ] Write failing trace tests for initial mount and action-request reconstruction,
  allowed mutation/await points, hook short-circuiting, panic/internal failure,
  teardown exactly once, suppressed downstream phases, and proof that separate
  requests use distinct owned Rust instances. Add failures for private authority,
  identity collision retry, capacity/clock/expiry, render/dehydrate/sign before
  ledger creation, ledger failure before publication, duplicate document-local
  identity, oversized metadata, and executable/script-bearing mount metadata.
- [ ] Define object-safe generated hooks using a bounded boxed future rather than
  retaining component objects:

  ```rust
  pub type LiveFuture<'a, T> =
      Pin<Box<dyn Future<Output = T> + Send + 'a>>;

  pub trait ComponentInstance: Send {
      fn metadata(&self) -> &'static ComponentMetadata;
      fn dehydrate(&self, exposure: StateExposure)
          -> Result<CanonicalValue, ComponentError>;
      fn render<'a>(&'a self, context: &'a RenderContext)
          -> LiveFuture<'a, Result<IslandRender, ComponentError>>;
  }
  ```

  Generated descriptor hooks create or hydrate one owned instance per request,
  execute only registered phases, and drop it after teardown.
- [ ] Add a distinct ledger request and operation:

  ```rust
  pub struct MountInstanceRecord {
      pub scope: ScopeFingerprint,
      pub instance_id: InstanceId,
      pub component_contract: ContentDigest,
      pub initial_revision: Revision,
      pub expires_at: UnixMillis,
  }

  async fn mount_instance(
      &self,
      record: MountInstanceRecord,
  ) -> Result<InstanceAuthority, LedgerError>;
  ```

  It is create-only and has no browser nonce or promotion idempotency recovery.
- [ ] Implement the private mount service in this order: validate trusted catalog
  entry and parameters; generate a candidate server identity; construct/mount the
  repeatable effect-free component under that identity; render all identity-bound
  island/child metadata; dehydrate and sign the instanced snapshot; validate the
  complete bounded island output; call `mount_instance`; then return publishable
  output. Retry only `InstanceConflict`, with a hard bounded attempt count, a
  fresh identity, and a repeated effect-free mount/render/sign but no domain
  effect.
- [ ] Keep public seed emission separate. A seed output has no instance ID,
  revision, or ledger record and contains only explicitly public state. On its
  later promotion, run the registered repeatable/effect-free mount initializer
  under current host context, then overlay only verified public fields; current
  defaults/host capabilities initialize every omitted category before proposals
  or actions can observe it.
- [ ] Make the engine own the single island wrapper and escape every attribute.
  Mount metadata is data only: component identity, document-local key, protocol
  minimum, signed envelope, and inert bounded flags.
- [ ] Add deterministic barriers/fault injection proving no observer receives
  publishable output before ledger authority exists.
- [ ] Run lifecycle, ledger, mount, and concurrency tests plus Clippy review.
- [ ] Commit: `feat: add Live lifecycle and atomic mounting`.

## Task 8: Implement nested composition and lazy server completion

**Files:**

- Create: `src/component/composition.rs`
- Create: `src/component/lazy.rs`
- Modify: `src/registry/descriptor.rs`
- Modify: `crates/suprnova-live-macros/src/live_impl.rs`
- Create: `tests/composition.rs`
- Create: `tests/lazy_components.rs`

- [ ] Write failing tests for typed mount/child parameters, duplicate/unstable
  keys, independent nested ownership, circular recursion, conditional removal,
  contract-changing remount, no-op parameter updates, bounded non-atomic pending
  state, child-only failure recovery, and lazy completion that never carries an
  unsolicited HTML patch.
- [ ] Implement developer stable keys, typed mount/child parameters, independent
  nested-island ownership, duplicate-key detection, circular depth/count bounds,
  conditional removal, remount on identity/contract change, and lazy placeholders
  whose server completion is a typed lifecycle result rather than streamed HTML.
- [ ] Define surviving-child states `Unchanged`, `PendingParams`, `Remount`, and
  `Removed`. Parent render is not transactionally atomic with a child request;
  failure recovery targets only the child. Produce an internal pending-parameter
  value here; Task 9 is the only place that turns it into browser-carried signed
  authority.
- [ ] Implement registered `params_changed` and `lazy_complete` lifecycle hooks
  as closed server operations. They reuse ordinary hydration/revision scheduling
  and cannot be invoked as arbitrary methods or streamed fragments.
- [ ] Run composition/lazy tests with injected clocks and barriers, never sleeps.
- [ ] Commit: `feat: add Live composition and lazy lifecycle`.

## Task 9: Sign child-parameter authority with its own capability

**Files:**

- Create: `src/child/mod.rs`
- Create: `src/child/schema.rs`
- Create: `src/child/codec.rs`
- Create: `src/child/verified.rs`
- Modify: `src/crypto/key.rs`
- Modify: `src/crypto/key_ring.rs`
- Modify: `src/crypto/signature.rs`
- Modify: `src/component/composition.rs`
- Create: `tests/child_parameter_envelope.rs`
- Create: `tests/child_parameter_tampering.rs`
- Create: `tests/child_parameter_properties.rs`

- [ ] Write failing tests for wrong purpose key, key ID, parent scope/instance/
  accepted revision, child key/component contract, parameter schema/value hash,
  issue/expiry bounds, superseded parent replay, duplicate keys, malformed
  canonical input, and raw browser parameter substitution.
- [ ] Add a purpose-derived `child-params-v1` HMAC key and canonical body:

  ```rust
  pub struct ChildParametersV1 {
      pub parent_scope: ScopeFingerprint,
      pub parent_instance: InstanceId,
      pub parent_revision: Revision,
      pub child_key: ChildKey,
      pub child_contract: ContentDigest,
      pub parameter_schema_version: u16,
      pub parameters: CanonicalValue,
      pub value_digest: ContentDigest,
      pub issued_at: UnixMillis,
      pub expires_at: UnixMillis,
      pub key_id: KeyId,
  }
  ```

- [ ] Verification returns `VerifiedChildParametersV1`; only that capability can
  expose typed values to the child's `params_changed` hook. A signed snapshot or
  trusted request context cannot substitute for it.
- [ ] Bind envelope issuance to the parent's accepted successor revision. A
  parent render can describe child changes before acceptance, but envelopes are
  not publishable until the matching parent outcome is accepted.
- [ ] Add canonical round-trip/property coverage and bounded fuzz regressions.
- [ ] Commit: `feat: add signed child parameter authority`.

## Task 10: Add validation, closed semantic outcomes, and registered actions

**Files:**

- Create: `src/validation/mod.rs`
- Create: `src/validation/error_bag.rs`
- Create: `src/validation/engine.rs`
- Create: `src/validation/port.rs`
- Create: `src/action/mod.rs`
- Create: `src/action/arguments.rs`
- Create: `src/action/outcome.rs`
- Create: `src/action/emission.rs`
- Create: `src/action/dispatch.rs`
- Modify: `src/metadata/method.rs`
- Modify: `src/component/executor.rs`
- Modify: `crates/suprnova-live-macros/src/live_impl.rs`
- Create: `tests/validation.rs`
- Create: `tests/action_dispatch.rs`
- Create: `tests/action_outcomes.rs`

- [ ] Write failing tests for unknown/private actions, malformed/oversized typed
  arguments, sync/async actions, selected/whole/action/cross-field validation,
  binding-versus-validation error separation, clear/retain/replace bag policy,
  current authorization ordering, unsafe redirect, unregistered event/effect,
  incompatible outcomes, panic, and redaction.
- [ ] Generate a closed action table keyed by validated `ActionName`; each entry
  owns argument metadata, authorization requirement, validation selection,
  transaction policy, and an erased typed dispatcher. Never reflectively invoke a
  method from browser text. Generated authoring docs state that action bodies must
  tolerate reinvocation before commit; external effects need their own
  idempotency, compensation, or outbox contract.
- [ ] Implement a validation engine that owns bounded localizable issue bags and
  delegates application/framework rules through `ValidationPort`. A conformance
  port proves ordering but is not called Suprnova validation integration.
- [ ] Implement closed outcomes:

  ```rust
  pub enum ActionOutcome {
      Render,
      NoRender,
      Redirect(RouteIntent),
  }

  pub struct OutcomeMetadata {
      pub flash: Vec<FlashIntent>,
      pub events: Vec<RegisteredEmission>,
      pub effects: Vec<RegisteredEmission>,
      pub url: Option<UrlIntent>,
  }
  ```

  Validate combinations before any response or ledger acceptance. Redirect wins
  and suppresses render; no outcome can carry arbitrary JavaScript or raw
  external URLs.
- [ ] Define `LiveEventPayload` and `LiveEffectPayload` with validated stable
  name, schema version, bounded canonical encoding, and safe diagnostics.
  `RegisteredEmission::from(payload)` succeeds only when the payload type occurs
  in the current component descriptor. Effect names are server metadata here;
  browser implementations remain Iteration 003.
- [ ] Reauthorize current protected resources through a host capability after
  verified hydration and before protected reads/effects. Snapshot contents never
  stand in for current authorization.
- [ ] Run targeted tests, macro UI tests, and Clippy review.
- [ ] Commit: `feat: add Live actions and validation`.

## Task 11: Coordinate host transactions and accepted ledger outcomes exactly

**Files:**

- Create: `src/execution/mod.rs`
- Create: `src/execution/service.rs`
- Create: `src/execution/transaction.rs`
- Create: `src/execution/trace.rs`
- Create: `src/execution/recovery.rs`
- Modify: `src/ledger/contract.rs`
- Modify: `src/promotion/service.rs`
- Modify: `src/component/executor.rs`
- Modify: `src/action/dispatch.rs`
- Modify: `tests/seed_promotion.rs`
- Create: `tests/execution_order.rs`
- Create: `tests/execution_fault_matrix.rs`
- Create: `tests/execution_concurrency.rs`

- [ ] Write a table-driven failure test with an injectable fault at every locked
  boundary: claim, hydrate, bind, authorize, validate, transaction begin, before,
  action, after, render, dehydrate, sign, outcome validation, host commit, ledger
  acceptance, and reporting. Assert exact trace, domain commit state, ledger
  state, response recovery, and whether retry is legal.
- [ ] Add the public-seed first-action matrix: promotion creates ledger authority,
  fresh mount initializes omitted state, verified public fields overlay according
  to policy, typed proposals apply afterward, and no action observes a partially
  reconstructed component. A mount failure consumes/requires safe refresh under
  Tier 0 and never falls back to browser-provided non-public fields.
- [ ] Refactor `PromotedInstance` so promotion returns ledger authority and an
  engine-internal `VerifiedSeedV1` capability rather than signing or exposing a
  partial instanced snapshot. The execution service alone signs the first
  browser-publishable instanced snapshot after full mount/overlay/proposal/action
  processing and instanced-schema validation. Without `refresh_on_promote`, apply
  the verified public overlay before proposals. With it, require protocol v2,
  accept a fresh-render outcome from current mounted values, and do not apply the
  original proposals or invoke the original action.
- [ ] Define host-neutral ports:

  ```rust
  pub trait HostTransaction: Send {
      fn commit(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>>;
      fn rollback(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>>;
  }

  pub trait TransactionPort: Send + Sync {
      fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>>;
  }
  ```

  A no-transaction policy is explicit metadata, not a fake successful
  transaction.
- [ ] Implement the exact Tier 0 order. Validate a complete successor snapshot,
  island render, emissions, redirect/URL intent, and safe response classification
  before committing the host transaction. Commit ledger acceptance only after
  the host transaction commits.
- [ ] If host commit succeeds and ledger acceptance fails, return
  `RefreshRequired` and permanently prohibit action replay for that request path.
  Reporting failures after acceptance are observable but cannot rewrite the
  accepted outcome.
- [ ] Retain bounded accepted metadata only. Exact compatible duplicates recover
  metadata; if response bytes are unavailable, return refresh-required without
  invocation. Durable after-commit delivery is exposed only as outbox/equivalent
  host intent, never an in-memory callback guarantee.
- [ ] Prove with deterministic concurrent requests that at most one committed
  Live outcome is accepted per base revision. Do not assert exactly-once method
  invocation or external effects.
- [ ] Commit: `feat: coordinate Live action outcomes`.

## Task 12: Add protocol v2 and semantic idempotency without changing v1

**Files:**

- Modify: `src/protocol/mod.rs`
- Modify: `src/protocol/request.rs`
- Modify: `src/protocol/response.rs`
- Modify: `src/protocol/compatibility.rs`
- Create: `src/protocol/idempotency.rs`
- Create: `src/protocol/v2.rs`
- Modify: `browser/src/protocol.ts`
- Modify: `browser/src/schema.ts`
- Modify: `browser/src/conformance.ts`
- Create: `fixtures/v2/*.json`
- Create: `fixtures/v2/manifest.sha256`
- Modify: `tests/golden_fixtures.rs`
- Modify: `browser/tests/golden-fixtures.test.ts`
- Create: `tests/protocol_v2.rs`
- Create: `tests/idempotency_digest.rs`

- [ ] Freeze current v1 fixtures first by recording their manifest and asserting
  their parsed/serialized behavior is unchanged. Add failing v2 fixtures for
  `params_changed`, `lazy_complete`, `fresh_render`, child envelope delivery,
  reflected/navigated URL intent, incompatible batching, and rolling-version
  rejection.
- [ ] Parse protocol version before version-specific fields, then dispatch to
  separate v1/v2 schemas. Do not add optional v2 meanings to v1. Add v2 operation
  variants only to the resolved versioned request model.
- [ ] Implement semantic digest profile v1 over:

  ```text
  profile + scope + instance + base revision + component contract
  + idempotency identity + snapshot/child authority digest
  + ordered operations + model proposals + semantic extensions
  ```

  Exclude correlation ID, header order, media formatting, transport extensions,
  and body whitespace. Changed authority, proposal field/value meaning, ordered
  operation, argument, or semantic extension must change the digest; object key
  presentation order must not.
- [ ] Extend response parsing/encoding for signed child deliveries and typed URL
  intent. Preserve the iteration-001 commit-after-morph ordering contract; v2
  `fresh_render` is recovery and never retries the original action.
- [ ] Make Rust and TypeScript enumerate `fixtures/v1` and `fixtures/v2` from one
  harness with exact manifests and no duplicate expected tables.
- [ ] Run Rust protocol/idempotency/golden tests and all TypeScript conformance
  checks with `rtk npm --prefix browser run format:check`, `lint`, `typecheck`,
  `test`, and `build`.
- [ ] Commit: `feat: add Live protocol v2 lifecycle operations`.

## Task 13: Build the host-neutral endpoint and typed HTTP response intent

**Files:**

- Create: `src/endpoint/mod.rs`
- Create: `src/endpoint/config.rs`
- Create: `src/endpoint/request.rs`
- Create: `src/endpoint/response.rs`
- Create: `src/endpoint/service.rs`
- Create: `src/endpoint/error.rs`
- Modify: `src/execution/service.rs`
- Modify: `src/mount/service.rs`
- Create: `tests/endpoint_contract.rs`
- Create: `tests/endpoint_failures.rs`
- Create: `tests/endpoint_duplicates.rs`
- Create: `tests/hostile_adapter.rs`

- [ ] Write failing cases for wrong method/media/charset/version, oversized body,
  cache attempts, missing/expired/inconsistent host context, route/slot/scope/
  catalog mismatch, incompatible batches, unsafe redirects, error redaction, and
  every success/rejected/conflict/duplicate/refresh/fatal mapping.
- [ ] Accept only normalized host-neutral input:

  ```rust
  pub struct LiveEndpointRequest {
      pub method: http::Method,
      pub content_type: ParsedLiveMediaType,
      pub body: bytes::Bytes,
      pub context: TrustedLiveRequestContext,
  }

  pub struct LiveEndpointResponse {
      pub status: http::StatusCode,
      pub headers: http::HeaderMap,
      pub body: bytes::Bytes,
  }
  ```

  Do not define a router, raw socket/body stream, cookies, forwarded-header
  parser, session middleware, or Suprnova request/response imitation.
- [ ] Validate transport and trusted context before seed promotion, instance
  claim, hydration, or action execution. Resolve the component only through the
  trusted catalog match and immutable registry.
- [ ] Own exact Live response media type, `Cache-Control: no-store`, bounded
  security headers, content length, and status mapping in the endpoint. Island
  metadata cannot override them. Encode complete bytes before returning success.
- [ ] Map duplicates without retained body to refresh-required. Map validation
  and authorization without leaking state or revealing whether a protected
  resource exists beyond policy.
- [ ] Run endpoint, execution, promotion, ledger, and hostile-adapter suites.
- [ ] Commit: `feat: add host-neutral Live endpoint service`.

## Task 14: Implement the branch-aware Askama and HTML checker

**Files:**

- Create: `src/checker/mod.rs`
- Create: `src/checker/template.rs`
- Create: `src/checker/html.rs`
- Create: `src/checker/branch.rs`
- Create: `src/checker/directive.rs`
- Create: `src/checker/diagnostic.rs`
- Create: `src/checker/limits.rs`
- Create: `tests/checker_positive.rs`
- Create: `tests/checker_negative.rs`
- Create: `tests/checker_regressions.rs`
- Create: `tests/fixtures/checker/pass/*.html`
- Create: `tests/fixtures/checker/fail/*.html`

- [ ] Write negative fixtures first for missing views, unknown components/actions/
  models, forbidden binding, invalid modifiers, duplicate/unstable keys, nested
  ownership violations, invalid lifecycle/URL metadata, raw `safe`, mismatched
  HTML stacks across `if`/`match`/`for`, dynamic tags/attributes, includes/
  inheritance failures, token/depth/count limits, and source-span stability.
- [ ] Parse template structure only with exact `askama_parser` 0.16. Walk nodes
  and control-flow branches while retaining source spans. Do not implement a
  second Askama grammar or validate only one rendered sample.
- [ ] Tokenize bounded static HTML regions with exact `html5ever` 0.39. Maintain
  an HTML/island/directive stack per branch and join only identical compatible
  states. A dynamic tag or attribute is `Unproved` unless it uses the one
  documented checked escape; it is never silently accepted as proved.
- [ ] Resolve directive values against generated registry metadata and the
  versioned server-visible grammar. Produce stable human and machine diagnostics
  with source location and safe registered identity only.
- [ ] Add property/fuzz regressions around both parser boundaries. Bound source
  bytes, template nodes, include depth, branch states, HTML tokens, attributes,
  stack depth, and diagnostic count before allocating unbounded collections.
- [ ] Run checker fixtures/properties/fuzz build and Clippy review.
- [ ] Commit: `feat: add checked Live template contracts`.

## Task 15: Build the host-neutral component harness and security matrix

**Files:**

- Create: `crates/suprnova-live-test-support/src/harness.rs`
- Create: `crates/suprnova-live-test-support/src/host.rs`
- Create: `crates/suprnova-live-test-support/src/trace.rs`
- Create: `crates/suprnova-live-test-support/src/assertions.rs`
- Create: `tests/component_harness.rs`
- Create: `tests/security_hostile_context.rs`
- Modify: `tests/parser_properties.rs`
- Modify: `tests/fuzz_regressions.rs`
- Modify: `tests/security_boundaries.rs`
- Create: `fuzz/fuzz_targets/protocol_v2_request.rs`
- Create: `fuzz/fuzz_targets/protocol_v2_response.rs`
- Create: `fuzz/fuzz_targets/child_parameters.rs`
- Create: `fuzz/fuzz_targets/template_checker.rs`

- [ ] Write a harness acceptance test first that mounts a representative
  component, supplies query/child/session parameters, proposes valid/invalid
  values, invokes an authorized action, injects a transaction failure, retries,
  and asserts HTML/state/error/event/effect/redirect/revision/snapshot/auth trace
  without a browser or Suprnova adapter.
- [ ] Implement controlled time, randomness, registry, host checks, mount catalog,
  session, authorization, validation, application services, transaction, ledger,
  and deterministic fault injection. Every assertion compares typed outcomes;
  snapshots and secrets remain redacted.
- [ ] Add the hostile-host matrix: absent/inconsistent/expired origin, CSRF,
  session, principal, tenant, proxy, rate-limit, route/slot, scope, and catalog
  facts; current authorization change; child-envelope replay; secret/transient
  redaction; and untrusted redirect/effect rejection.
- [ ] Extend parser properties and fuzz targets for every newly exposed parser or
  verifier. Persist all discovered regressions as deterministic tests and prove
  no hostile-input panic.
- [ ] Run all harness/security tests and bounded nightly fuzz smoke campaigns.
- [ ] Commit: `test: add Live component and host conformance harness`.

## Task 16: Measure macro/checker/action budgets and wire the unattended gate

**Files:**

- Create: `benches/action_framework_budget.rs`
- Create: `benchmarks/action-budget-v1.json`
- Create: `scripts/run-action-budget.sh`
- Create: `scripts/check-expansion-budget.mjs`
- Create: `benchmarks/expansion-budget-v1.json`
- Create: `tests/fixtures/compile/1-component/*`
- Create: `tests/fixtures/compile/10-components/*`
- Create: `tests/fixtures/compile/100-components/*`
- Modify: `tests/benchmark_contract.rs`
- Modify: `tests/gate_contract.sh`
- Modify: `scripts/gate.sh`
- Modify: `scripts/generate-license-inventory.mjs`
- Modify: `THIRD_PARTY_LICENSES.md`

- [ ] Add failing budget/gate contract tests first. The gate must contain both
  iteration budget runners, macro UI and checker fixtures, v1/v2 Rust/TypeScript
  parity, security tests, nightly fuzz build, MSRV checks, and license/spec gates;
  it must reject blanket `-D warnings`.
- [ ] Implement A8/16 action-framework timing from accepted request bytes through
  parse/verify/claim/hydrate/bind/dispatch framework phases and successor
  classification, excluding application action body, domain/provider I/O, and
  Askama render time. Record at least 30 post-warmup samples and fail local p95 at
  or above 2 milliseconds.
- [ ] Record CPU, selected affinity, memory, kernel, governor, rustc/profile,
  warmups/samples, fixture digest, p50/p95, and environment qualification. Keep
  local exploratory evidence honest; validated S1 remains release qualification.
- [ ] Measure expanded token/byte count and isolated `cargo check` work for fixed
  1-, 10-, and 100-component fixtures. Reject unexplained superlinear growth and
  retain checked baselines. Never market local measurements as release-grade.
- [ ] Keep the complete L0 counting-allocator harness from Iteration 001 intact
  and add no blanket warning denial. Review every Clippy warning mechanically in
  the gate output.
- [ ] Run the targeted budget scripts and gate contract.
- [ ] Commit: `perf: gate Live kernel budgets`.

## Task 17: Document the implemented kernel and regenerate the spec handoff

**Files:**

- Modify: `README.md`
- Create: `docs/implementation/component-authoring.md`
- Create: `docs/implementation/views-and-checker.md`
- Create: `docs/implementation/lifecycle-and-state.md`
- Create: `docs/implementation/actions-and-validation.md`
- Create: `docs/implementation/host-adapter-contract.md`
- Create: `docs/implementation/protocol-v2.md`
- Create: `docs/implementation/component-harness.md`
- Modify: `docs/implementation/fixtures.md`
- Modify: `docs/implementation/benchmarking.md`
- Modify: `docs/implementation/threat-model-v1.md`
- Modify: `docs/specs/suprnova-live/*.md` only for implementation facts
- Modify: `docs/specs/suprnova-live.zip`

- [ ] Add a failing documentation/gate assertion for every required authoring,
  metadata, lifecycle, state, binding, action, validation, rendering, trusted
  context, endpoint, security, harness, fixture, failure/recovery, and benchmark
  section.
- [ ] Document final application-facing examples with `suprnova::live` paths,
  while labelling the macro crate/facade fixture and host adapters as internal
  standalone development machinery. Do not present conformance as registered
  Suprnova integration.
- [ ] Record exact protocol v2, child envelope, mount authority, state category,
  idempotency, endpoint, and transaction behavior. State plainly that Live
  guarantees at most one accepted committed outcome per base revision, not
  exactly-once method invocation or external effects.
- [ ] Update normative decision logs only where implementation resolved a real
  choice. Keep `Last revised` synchronized and entries newest-first. Add glossary
  terms only if their meaning was confirmed rather than merely convenient.
- [ ] Regenerate the optional Fable archive exactly:

  ```bash
  rtk proxy bash -lc 'cd docs/specs && zip -X -q -FS -r suprnova-live.zip \
    suprnova-live -i "*.md" -x "suprnova-live/iterations/next/*"'
  rtk node scripts/check-specs.mjs
  ```

- [ ] Run link checks, `rtk git diff --check`, license inventory, and all doc
  tests.
- [ ] Commit: `docs: record Live server-component kernel`.

## Task 18: Run the complete Iteration 002 gate and final self-audit

**Files:**

- Review: every tracked Iteration 002 file
- Modify: only defects proven by checks or self-audit

- [ ] Run `rtk node scripts/check-specs.mjs`.
- [ ] Run `rtk git diff --check`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo fmt --all --check`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets
  --all-features` and review every warning without `-D warnings`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --workspace --all-targets
  --all-features --no-fail-fast`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 cargo test --workspace --doc
  --all-features`.
- [ ] Run macro trybuild and checker positive/negative fixtures explicitly.
- [ ] Run `rtk npm --prefix browser ci`, then the format, lint, typecheck, test,
  build, and budget scripts through `rtk npm --prefix browser run ...`.
- [ ] Run `rtk env CARGO_INCREMENTAL=0 scripts/gate.sh`.
- [ ] Run the pinned MSRV test and Clippy matrix without blanket warning denial.
- [ ] Run `rtk cargo +nightly fuzz build` and bounded smoke campaigns for every
  Iteration 001 and 002 target.
- [ ] Run both budget scripts and verify honest environment labels/sample counts.
- [ ] Inspect the complete diff and tracked inventory. Search for placeholders,
  `unsafe`, warning denial, unbounded external input, raw `safe`, secret-bearing
  formatting, global inventory, sticky component state, response-body ledger
  storage, development macro paths, accidental browser-runtime work, or claims
  of completed Suprnova integration.
- [ ] Map evidence to every completion condition below. Re-run every check touched
  by remediation; do not mark complete on stale output.
- [ ] Inspect only the status and current commit of the active Suprnova and
  Magnetar worktrees. Confirm no Iteration 002 command wrote to either repository.
- [ ] Commit the verified final state locally, do not push:
  `feat: complete Suprnova Live iteration 002`.

## Definition-of-done coverage matrix

| Iteration 002 condition | Primary tasks | Required evidence |
| --- | --- | --- |
| 1. Reproducible engine/macro workspace | 1, 18 | workspace contract, lock/license checks, MSRV/gate |
| 2. Versioned metadata and diagnostics | 2, 3 | metadata/registry tests, trybuild spans, final-path expansion |
| 3. Checked Askama rendering | 6, 14 | escaping/trusted-markup tests, checker fixtures, no partial output |
| 4. Initial mount authority and island roots | 7 | mount/ledger fault matrix and publication barrier |
| 5. Lifecycle order and stateless reconstruction | 7 | lifecycle traces, failure suppression, object reconstruction proof |
| 6. Typed child parameters and stable keys | 8, 9, 12 | signed-envelope tampering/replay and composition-state tests |
| 7. Every state category enforced | 3, 4 | schema/snapshot/leakage/compile-fail tests |
| 8. Lossless typed model binding | 4 | scalar/collection/nested/property tests and hostile paths |
| 9. Timing and URL metadata agreement | 3, 4, 12, 14 | macro/checker/runtime fixtures and typed URL outcomes |
| 10. Closed registered action dispatch | 3, 10 | trybuild, dispatch, malformed-operation, panic tests |
| 11. Validation and error bags | 10 | selected/whole/action/cross-field and policy tests |
| 12. Current authorization | 5, 10, 13, 15 | hostile context and changed-permission traces |
| 13. Transactions, ledger, duplicate/fresh recovery | 11, 13 | deterministic fault/concurrency/duplicate matrices |
| 14. Closed semantic outcomes and ordering | 10-12 | outcome compatibility and v1/v2 ordering fixtures |
| 15. Endpoint transport contract | 13 | method/media/cache/size/status/header/recovery matrix |
| 16. Non-browser-constructible trusted context | 5, 15 | production API compile checks and hostile adapter suite |
| 17. Askama-aware branch checker | 14 | positive/negative/regression/property/fuzz fixtures |
| 18. Browserless component harness | 15 | representative end-to-end harness acceptance test |
| 19. Complete hostile/parser verification | 3, 4, 9, 12, 14, 15, 18 | unit/UI/integration/property/fuzz/concurrency gate |
| 20. A8/16 action budget | 16, 18 | versioned result with >=30 samples and local p95 <2 ms |
| 21. Expansion/check scaling | 3, 14, 16 | fixed 1/10/100 fixture baselines and regression gate |
| 22. Unattended gate without `-D warnings` | 16, 18 | gate contract plus successful full run |
| 23. Complete implementation documentation | 17 | doc contract, links, examples, benchmark reproduction |
| 24. No drift/placeholders/archive mismatch | 17, 18 | spec checker, archive equality, drift search |
| 25. Active repos untouched and no push | every task, 18 | read-only status/commit comparison and local-only final commit |
