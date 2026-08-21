# Suprnova Live -- 06 Wire Protocol and Transport

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the versioned request and response protocol between the Live
browser runtime and Suprnova Live endpoints, including operation envelopes,
correlation, batching, result ordering, error taxonomy, and compatibility. It
depends on snapshots and actions and feeds browser scheduling, effects,
navigation, and diagnostics. Security policy and file-transfer mechanics have
their own specs.

## Capabilities

### Endpoint and media contract

Live interactions shall use explicit Suprnova HTTP endpoints and versioned media
contracts distinct from ordinary application routes. The endpoint shall accept
only intended methods, content types, sizes, and protocol versions.

Acceptance criteria:
- Endpoint discovery is emitted through framework-owned configuration or
  metadata without hard-coded application URLs.
- State-changing operations use non-GET semantics and integrate with normal
  middleware ordering.
- Request and response content types and character encoding are explicit.
- Unsupported method, media type, version, or size produces a classified HTTP
  and protocol error.
- Endpoint responses are not cached as canonical documents.
- Transport configuration works behind route prefixes and trusted reverse
  proxies without browser-generated authority.

UX flow:
1. Runtime connects an island -> it resolves the compatible endpoint and
   protocol metadata.
2. Endpoint configuration is incompatible -> initial HTML remains and the
   island exposes a runtime compatibility failure.

### Update request envelope

An update request shall identify its island and component contract and carry
either a current instanced snapshot or, for the first action on a public cached
island, a seed snapshot plus a proposed instance nonce. It shall also carry the
applicable base revision, correlation and idempotency data, allowed model
proposals, and one or more declared operations. Browser data shall remain
structurally separate from server-trusted memo.

Acceptance criteria:
- Required and optional fields are explicitly versioned.
- Model proposals, action arguments, and metadata have independent schemas and
  limits.
- Seed promotion input is a distinct operation form whose browser nonce remains
  untrusted until atomic instance creation succeeds.
- A request cannot select an unregistered component type or Rust method.
- Correlation identifiers are safe to log and do not contain secrets.
- Duplicate keys, ambiguous operations, and invalid ordering are rejected.
- Batching is allowed only where operations share a compatible island,
  snapshot, security context, and deterministic execution order.

UX flow:
1. Runtime schedules compatible work -> it creates one bounded request envelope
   with the current instanced snapshot or eligible first-action seed operation.
2. Work cannot be safely batched -> requests remain separately ordered rather
   than changing semantics to reduce traffic.

### Update response envelope

A response shall identify the request and accepted revision and carry the new
snapshot, rendered island HTML when applicable, validation data, declared
events or browser effects, redirect information, and classified errors. Result
application shall follow one protocol state machine rather than an
implementation-dependent ordering.

Acceptance criteria:
- A success cannot omit required revision or snapshot data unless its declared
  outcome explicitly makes them unnecessary.
- HTML, snapshot, errors, effects, and redirect fields cannot be confused by
  content sniffing or arbitrary script evaluation.
- Redirects target real navigation and use validated locations.
- Partial per-operation results are represented only when transaction and
  action semantics permit them.
- Runtime can distinguish accepted, rejected, retryable, refresh-required, and
  fatal outcomes.
- Response sizes are bounded and measured before browser application.
- The runtime validates the complete envelope, correlation, and expected
  revision before applying any field.
- A valid redirect is terminal: the runtime skips morph, snapshot installation,
  events, and effects and performs real document navigation.
- Otherwise the runtime preflights and performs the morph, then atomically
  installs the new snapshot and accepted browser revision, reconciles
  model/validation metadata and focus, dispatches events, runs registered
  effects, and finally settles feedback state.
- An explicit no-render outcome substitutes successful no-render validation for
  the morph phase before snapshot/revision installation.
- Morph failure after server acceptance leaves the browser on its prior
  snapshot and requires fresh rendering; the original action is never retried.

UX flow:
1. Server accepts a non-redirect interaction -> runtime preflights, morphs, then
   commits browser state and applies metadata, events, effects, and feedback in
   the defined order.
2. Response is malformed or mismatched -> runtime applies none of the uncertain
   output and enters protocol recovery.

### Correlation, idempotency, and duplicate handling

Every interaction shall be correlatable end to end, and operations that may be
retried shall carry an idempotency identity with a defined scope and lifetime.
Transport retry shall not silently duplicate committed Live outcomes.

Acceptance criteria:
- Request identifiers, island revisions, and idempotency keys serve distinct
  documented purposes.
- Duplicate accepted requests return the prior compatible outcome or a
  classified duplicate response without rerunning protected effects.
- At most one committed Live outcome is accepted for an instance base revision;
  a Rust method may run again after a transaction that committed neither claim
  nor effect.
- Idempotency storage is bounded and scoped to the correct principal/tenant,
  instance, component, and action.
- Non-idempotent work is not automatically retried merely because the network
  response was lost.
- Logs and traces can correlate browser, endpoint, action, render, and response.

UX flow:
1. A retryable request loses its response -> runtime retries with the same
   idempotency identity under policy.
2. Server has already committed it -> duplicate handling returns the accepted
   result or a classified duplicate without committing a second outcome.

### Error taxonomy and recovery instructions

Protocol errors shall be stable machine-readable categories paired with safe
application-user behavior and detailed developer diagnostics. HTTP status,
protocol category, and recovery instruction shall agree.

Acceptance criteria:
- Categories cover validation, binding, authentication, authorization, CSRF,
  rate/size limits, stale or expired snapshot, conflict, action failure, render
  failure, compatibility, and internal failure.
- Each category declares whether to retain DOM, retry, refresh island, navigate,
  or stop.
- Production messages avoid stack traces, secrets, policy internals, and raw
  state.
- Development diagnostics preserve correlation and originating source where
  available.
- Unknown errors fail closed with bounded recovery.

UX flow:
1. Server rejects or fails a request -> runtime receives one classified recovery
   instruction.
2. Runtime follows it -> current DOM is retained whenever safe and no rejected
   effect is represented as successful.

### Compatibility and deployment evolution

Protocol evolution shall support explicit compatibility windows and predictable
recovery across rolling deployments. The runtime and server shall never guess
that incompatible messages are safe.

Acceptance criteria:
- Runtime asset and server protocol versions are observable.
- Backward-compatible additions and breaking changes have separate rules.
- Rolling nodes either share a compatible window or reject with refresh
  instructions.
- Removed behavior cannot be invoked through stale metadata indefinitely.
- Compatibility fixtures exercise supported old/new runtime-server pairs.

UX flow:
1. Application user retains an old document during deployment -> its runtime
   either communicates within the compatibility window or receives one bounded
   refresh instruction.
2. Refresh obtains current assets and document metadata -> Live resumes without
   repeated incompatibility loops.

## Iteration 001 implementation profile

The checked parser and pure application model are documented in
[`protocol-v1.md`](../../implementation/protocol-v1.md). A request has exactly
the v1 protocol/runtime/snapshot versions, correlation and idempotency
identities, component, decimal-string base revision, one distinct snapshot
form, bounded model proposals, ordered operations, and namespaced extensions.
Snapshot forms are `instance` or `seed_promotion`; the latter also requires an
at-least-128-bit browser nonce and base revision zero. Parsing validates syntax
and batch order but deliberately performs no snapshot verification, component
registry lookup, action dispatch, authentication, or authorization.

The checked response outcomes are `accepted`, `duplicate`, `rejected`,
`refresh_required`, and `fatal`. Accepted/duplicate output is either a committed
snapshot/revision with explicit HTML or no-render state, or a structurally
exclusive same-origin route redirect. Retry is represented by a rejected safe
error with the `retry` recovery instruction rather than a sixth outcome.
Nonaccepted responses cannot carry committed state, events, effects, or
redirects.

The application planner is a semantic model, not a DOM runtime. Redirect is
terminal. HTML preflight and morph precede browser snapshot/revision commit;
no-render validation occupies the same gate. Reconciliation, focus, events,
registered effects, and feedback follow commit. A post-acceptance morph failure
requests fresh rendering without replay. Iteration 002 owns the host-neutral
endpoint/media service contract; the atomic integration move owns its actual
Suprnova HTTP/middleware adapter, and iteration 003 owns scheduling and real DOM
execution.

## Iteration 002 implementation profile

Iteration 002 turns the v1 parser and response model into a host-neutral Live
endpoint service and adds protocol v2 for the server-component operations that
v1 cannot represent. Protocol v1 remains accepted for its existing model-sync
and action shapes. Protocol v2 adds typed `params_changed`, `lazy_complete`, and
`fresh_render` lifecycle operations plus bounded child-parameter and URL-intent
response fields; snapshot schema v1 remains independent. A component contract
declares the minimum protocol/runtime contract it requires, and rolling nodes
never reinterpret a v2 operation as v1.

A Live host adapter supplies bounded body bytes, normalized method/media/version
facts, endpoint configuration, and a trusted Live request context; the service
verifies the snapshot or child envelope, resolves generated component and
operation metadata, applies models, dispatches the action or lifecycle
operation, coordinates ledger and host transaction semantics, renders, signs,
and returns typed HTTP response intent.

The service defines exact method, content type, cache prohibition, status,
header, body-size, compatibility, duplicate, and recovery behavior. It does not
bind the active Suprnova `Router`, `Request`, middleware stack, or `Response`.
Only an adapter implemented during the atomic integration move may make that
claim; iteration 002 conformance adapters prove the normalized boundary and
reject missing or inconsistent host attestations.

The idempotency request digest has its own versioned canonical profile. It
includes the scoped instance, base revision, component contract, idempotency
identity, snapshot/child authority digest, ordered operations, model proposals,
and semantic extensions while excluding correlation IDs and transport-only
metadata. A retry may change correlation but cannot change meaning. Because the
instance ledger stores bounded accepted metadata rather than response bodies, a
duplicate whose full response is unavailable returns refresh-required without
rerunning the action; iteration 002 does not add a hidden component or response
blob store merely to replay bytes.

## Acceptance criteria

- Wire requests and responses are versioned, bounded, correlated, and
  structurally unambiguous.
- Operation ordering and batching never change action semantics.
- Duplicate and retry behavior protects durable effects.
- Errors carry safe deterministic recovery instructions.
- Rolling deployments have an explicit compatibility path.

## Decisions and revisions

- 2026-08-21 -- Kept protocol v1 stable and introduced protocol v2 for
  `params_changed`, `lazy_complete`, `fresh_render`, child-parameter delivery,
  and URL intent. Defined a semantic idempotency-digest profile that excludes
  correlation, and chose refresh-without-replay when metadata-only duplicate
  authority cannot reproduce a prior response body.
- 2026-08-21 -- Assigned the host-neutral v1/v2 endpoint service and typed HTTP
  intent to iteration 002. Actual Suprnova router/request/response and
  middleware binding remains reserved for the atomic integration move.
- 2026-08-21 -- Recorded the implemented v1 request/response field profiles,
  exact closed outcomes, parser-without-dispatch boundary, and pure
  commit-after-morph application model.
- 2026-08-21 -- Chose a versioned JSON control protocol for inspectability and
  ecosystem simplicity; binary file payloads use the upload contract.
- 2026-08-21 -- Real redirects navigate to routes; rejected an SPA page protocol
  inside the Live wire format.
- 2026-08-21 -- First action may promote a signed public seed using an untrusted
  browser nonce. Ordinary requests use instanced snapshots and base revisions.
- 2026-08-21 -- Locked response application order: redirect is terminal;
  otherwise morph succeeds before browser snapshot/revision commit, followed by
  reconciliation, events, effects, and feedback.
- 2026-08-21 -- Idempotency guarantees one committed outcome per base revision,
  not exactly-once method invocation or external effects.
