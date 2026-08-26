# Suprnova Live -- 07 Security and Trust Boundaries

Status: Normative design specification
Last revised: 2026-08-26

## Scope

This domain owns the Live threat model and enforceable trust boundaries across
HTML, snapshots, actions, model proposals, identity, authorization, tenants,
runtime assets, and operational diagnostics. It depends on Suprnova's existing
security facilities but does not assume their correctness without integration
verification. It constrains every other Live domain and is cross-referenced
rather than duplicated by them.

## Capabilities

### Explicit client distrust

All browser-provided state, identifiers, action names, arguments, URLs,
revisions, and metadata shall be treated as untrusted. A valid signature proves
only integrity of the signed fields issued by Suprnova; it does not prove
current authorization, secrecy, freshness, or domain validity.

Acceptance criteria:
- Threat modeling covers tampering, replay, confused deputy, mass assignment,
  cross-island substitution, cross-user substitution, XSS, CSRF, injection,
  tenant leakage, resource exhaustion, and stale authority.
- Security-sensitive checks identify their trusted source of truth.
- Locked or signed identifiers are reauthorized before protected use.
- Browser-hidden HTML and JavaScript variables are never considered secrets.
- Fuzz and negative tests exercise every external parser and dispatcher.

UX flow:
1. Hostile or malformed input reaches a boundary -> validation rejects it before
   protected work.
2. Legitimate request fails a changed authority check -> no effect occurs and
   the application follows its safe recovery surface.

### Snapshot signing and key management

Instanced snapshots and public seed snapshots shall use purpose-separated,
modern integrity constructions with constant-time verification where
applicable, explicit key identifiers, rotation, and bounded acceptance windows.

Acceptance criteria:
- Signing keys are separate from session, CSRF, encryption, and application
  cache keys.
- Instanced canonical bytes include component, island instance,
  identity-binding, revision, version, and expiry fields required by policy.
- Seed canonical bytes include component/build contract, route, island slot,
  public parameters/state, bounded issue age, and advisory generations while
  excluding an already-authoritative instance identity.
- Verification occurs before hydration and expensive processing.
- Key rotation permits an intentional overlap window and retires old keys.
- Missing, unknown, weak, or malformed key configuration fails closed.
- Secrets never appear in snapshots, HTML, logs, metrics, or client errors.

UX flow:
1. Valid snapshot arrives during key rotation -> a permitted verification key
   accepts it and the next response uses the current signing key.
2. Signature cannot be verified -> no action runs and the island receives a
   fresh-render instruction or terminal security error.

### Request authenticity and session integration

Live endpoints shall use Suprnova's CSRF, origin, cookie, session, TLS/proxy,
and middleware contracts with ordering proven by integration tests. Long-lived
documents and concurrent tabs shall not bypass current session validity.

Acceptance criteria:
- State-changing requests require the configured CSRF proof and accepted origin
  policy.
- Cookies retain appropriate secure, HTTP-only, same-site, path, and domain
  properties through Live routing.
- Trusted proxy configuration cannot be inferred from arbitrary forwarded
  headers.
- Session expiry, logout, rotation, and principal change invalidate incompatible
  identity-bound work.
- Seed promotion requires the same current CSRF, origin, session, tenant, and
  middleware checks as an ordinary first action.
- Cookie-authorized WebSocket upgrades reject missing, null, or unapproved
  `Origin` before accepting the connection. Explicit cross-origin streaming
  requires a configured non-wildcard allowlist and a separate transport
  credential contract; an opaque document-transport handle is never authority.
- Cross-origin use is denied unless an explicit supported deployment contract
  enables it.
- Browser transport controls and a non-authoritative document handle never
  grant membership authority. External SSE/WebSocket add and remove re-evaluate
  exclusive descriptor expiry plus current component contract, principal,
  session, tenant, aggregate scope, active membership, stream, resolved topics,
  event contracts, registered modes, origin, document, and operation through a
  trusted host port. Source subscription is bracketed by preflight and
  post-await validation; failure after opening closes/disposes the new logical
  session once and commits nothing.

UX flow:
1. Authenticated application user invokes an action -> current middleware
   resolves session, CSRF, and principal before Live dispatch.
2. Session is expired or rotated -> action is rejected and the application
   exposes its sign-in or refresh path without losing authority boundaries.

### Authorization and tenant isolation

Every protected component mount, model read, action, upload, broadcast
subscription, fresh render, and private cache composition shall authorize the
current principal against the current resource and tenant.

Acceptance criteria:
- Authorization is re-evaluated after hydration using server-authoritative
  resource lookup.
- Mount permission does not imply perpetual action or refresh permission.
- Tenant identifiers originate from trusted routing or identity context and are
  validated against every resource boundary.
- Private errors and HTML cannot cross principal or tenant variance.
- Authorization denial is distinguishable internally from not-found while the
  public response follows disclosure policy.
- Tests cover identity changes between render, action, refresh, push event, and
  cache stitching.
- Upload create/chunk/status/cancel/finalize and stream establish/renew/resume
  each reauthorize their exact current scope; a handle, descriptor, sequence, or
  browser nonce is never treated as sufficient authority.

UX flow:
1. Principal attempts a permitted operation -> current authorization allows the
   action or render.
2. Resource or permission moved to another tenant/context -> Live reveals no
   protected data and returns the declared denial recovery.

### Output and browser security

Rendered HTML, runtime configuration, effects, URLs, and third-party controller
integration shall preserve output escaping, Content Security Policy, safe URL,
and script-execution boundaries.

Acceptance criteria:
- Template interpolation escapes by default and trusted HTML requires explicit
  reviewable construction.
- Live never executes arbitrary JavaScript strings returned by an action.
- Browser effects use a registered data protocol rather than `eval` or inline
  code generation.
- Redirects and asset URLs reject unsafe schemes and header injection.
- Runtime assets support nonce/hash or external-script CSP deployment.
- Morphing does not cause inert or untrusted script markup to execute
  unexpectedly.

UX flow:
1. Application renders untrusted content -> it appears as content, not
   executable markup.
2. Action requests a browser behavior -> only a registered effect with validated
   data is dispatched.

### Abuse resistance and safe diagnostics

Live shall bound CPU, memory, payload, nesting, operation count, request rate,
and error disclosure before hostile traffic can amplify component work.

Acceptance criteria:
- Configurable global, route, component, principal, and operation limits have
  secure defaults.
- Limits apply before hydration, uploads, database work, and rendering where
  possible.
- Rate limiting and idempotency cannot be bypassed by changing untrusted island
  identifiers.
- Seed promotion is limited by source, principal/session or supported anonymous
  context, route/component, and outstanding-instance volume so a reusable seed
  cannot exhaust ledger storage.
- A browser-proposed instance nonce is never trusted as authorization,
  principal identity, uniqueness proof, or permission to join an existing
  instance.
- Logs redact state values, cookies, signatures, CSRF tokens, transfer grants,
  subscription credentials, upload handles where linkability is sensitive, and
  action arguments.
- Upload and stream limits cover aggregate bytes, temporary storage, validation
  time, connections, messages, replay, fanout, buffers, reconnects, and
  cleanup, not merely one request body.
- Async delivery buffers accept only framework-sealed entries minted after
  fresh exact document membership, signed-descriptor binding, current
  authorization memo/registry, expiry, revocation, event contract, and trusted
  resolved-target checks. Browser values and buffer callers cannot supply
  recipient counts, target-set scope, or reusable admission authority. Raw
  entries and offer/replay methods are not public capabilities: one
  document-owned operation performs final current-host revalidation and
  synchronous queue commit without an await or caller callback between them.
  Dequeued work remains inside a non-cloneable RAII lease; the document, not a
  caller, selects the exact binding's existing sequence machine and invokes the
  registered dispatcher. Denial, cancellation, dispatch failure, or unresolved
  drop cannot be reported as successful delivery.
- The browser async artifact accepts only membership-decoded envelopes through
  one exhaustive presentation dispatcher over a slot-specific core-owned async
  island port. That runtime object omits upload, model, component-state, generic
  registration, action, effect, call, HTML, snapshot, revision, and generic-write
  authority. Core snapshots one bounded current descriptor registration from own
  data properties before validation, mints the opaque capability, and rechecks the
  exact island owner, current capability, and each guarded target's connected
  current owner/scope after event construction and immediately before every
  bounded DOM dispatch. Presentation-signal authority binds the signed stable
  alphanumeric-first signal-scope identity, signal name, and local-signal-only
  null/boolean/string/safe-integer type; core rechecks the exact connected scope
  element immediately before the write. No selector, raw DOM target, nearest-
  scope fallback, or island-root default crosses the optional-feature boundary.
- Metrics use bounded labels and do not create attacker-controlled cardinality.
- Upload cleanup metrics are limited to closed age, retained-volume, outcome,
  retry, and orphan buckets. They never carry upload handles, lease identities,
  filenames, paths, scopes, topics, principals, grants, or raw errors, and an
  observer failure cannot rewrite cleanup authority.
- Security failures retain correlation identifiers suitable for investigation.

UX flow:
1. Legitimate application user reaches a limit -> accessible feedback states
   whether and when retry is possible.
2. Hostile traffic exceeds policy -> requests are rejected cheaply and safely
   without exposing enforcement internals.

## Iteration 001 implementation profile

The implemented boundary and deferred integrations are enumerated in
[`threat-model-v1.md`](../../implementation/threat-model-v1.md). Iteration 001
treats canonical bytes, signed envelopes, embedded key IDs, signatures,
browser nonces, component/action/model identities, arguments, revisions,
correlation/idempotency values, responses, redirects, errors, and extensions as
untrusted. Trusted inputs are registered schemas, framework-supplied binding
expectations, configured root keys/windows, injected clocks, server randomness,
provider configuration, and provider-issued opaque claim tokens.

Every external parser/verifier is byte/count/depth bounded and covered by
property tests, persisted hostile regressions, and a nightly fuzz target.
Snapshot/request debug output is redacted. Production errors use closed
category/recovery/detail enums. Telemetry accepts only closed event/outcome/error
dimensions plus an optional fixed-width digest prefix, never raw identities or
payloads.

These controls prove integrity, compatibility, binding, revision arbitration,
promotion abuse bounds, and safe classification. They do not implement or
stand in for TLS/proxy policy, origin/CSRF checks, cookies/sessions, principal
or tenant resolution, current authorization, domain freshness, HTTP dispatch,
CSP, DOM morphing, or browser effect execution. Iteration 002 owns the trusted
server host contract and kernel enforcement; the atomic integration move owns
actual Suprnova adapters, and iteration 003 owns browser/output integrations.

## Iteration 002 implementation profile

Iteration 002 defines a non-browser-constructible trusted Live request context
and requires it before promotion, hydration, model application, action dispatch,
fresh render, or endpoint success. The capability records only normalized,
bounded facts and opaque host handles needed by the kernel; it never carries
raw cookies, CSRF tokens, session secrets, forwarded headers, or reusable
authorization results. Component and action authorization still runs against
current host authority after verified hydration and before protected work.

Production construction belongs exclusively to the eventual Suprnova host
adapter after its origin, CSRF, session, principal, tenant, proxy, rate, and
middleware checks pass in proven order. Iteration 002 supplies private
conformance/test builders and hostile-adapter suites that prove the kernel
rejects absent, inconsistent, expired, cross-principal, cross-tenant, and
cross-route context. These are security contract tests, not claims that the
active Suprnova checkout is integrated.

Iteration 002 removes the public zero-input
`PromotionAttestations::verified()` assertion from production boundaries. The
host adapter must provide typed dispositions for every configured authenticity
check (`passed` or policy-declared `not_required`), a current scope fingerprint,
and a bounded mount-catalog match. An `unchecked` or missing disposition cannot
construct endpoint authority. This is defense against accidental adapter
omission, not a claim that the engine can prove a trusted host is truthful;
actual middleware-order verification remains an integration test responsibility.
Test construction lives in a dev-only harness dependency rather than a
production feature or public convenience constructor.

## Acceptance criteria

- Live has a documented threat model with tests for every external trust
  boundary.
- Snapshot integrity, request authenticity, current authorization, and secrecy
  remain separate concepts.
- Tenant and principal isolation cover actions, refreshes, uploads, pushes, and
  cache composition.
- Browser output and effects cannot introduce arbitrary script execution.
- Resource limits and diagnostics resist abuse without leaking sensitive data.

## Decisions and revisions

- 2026-08-26 -- Closed browser push dispatch to one exhaustive validated-envelope
  router with exactly refresh, registered browser-event, and declared local-signal
  presentation effects. Bound each core-minted event capability to its exact
  island port and repeated owner/current-capability checks after target resolution,
  preventing forged, stale, cross-island, retired, or resolver-raced authority from
  reaching DOM dispatch.
- 2026-08-25 -- Removed the public seal-to-offer and pop-to-success trust gaps
  from async delivery. Only the document owner may perform final admission and
  closed registered dispatch; raw sealed entries, offers, and delivery leases
  remain private, and every unresolved lease degrades continuity while releasing
  its shared permit.
- 2026-08-25 -- Made physical transport membership a fresh host-authority
  boundary rather than a retained descriptor check. The framework owns the
  comparison sink, independently checks exclusive expiry before and after each
  asynchronous authority lookup, requires exact current authorization memo,
  stream, topics, full event contracts, and canonical mode-set agreement, and
  supplies the exact operation/origin/document/subscription facts to the host. A
  second attempted current-snapshot acceptance fails closed rather than
  replacing the first decision.
  Add revalidates after source subscription and disposes an opened session on
  drift; external remove reauthorizes while internal retirement remains safe.
- 2026-08-25 -- Implemented exact normalized HTTP(S) WebSocket origin policy
  before descriptor or credential processing. Missing, duplicate, opaque,
  wildcard, malformed, userinfo-bearing, path/query-bearing, and unapproved
  origins fail closed; cross-origin use requires both an exact finite allowlist
  entry and separately authenticated non-cookie authority. Transport errors,
  frames, descriptors, credentials, and document handles expose only bounded
  redacted diagnostics.
- 2026-08-25 -- Demoted the cloneable async envelope context to a static
  authorization/codec contract. Every queue admission now rechecks exclusive
  descriptor expiry, active host membership, exact subscription and stream,
  current full event contracts, declared presentation signals, and the closed
  payload before issuing a non-cloneable one-use guard. Only the sequence
  machine consumes that guard and rechecks descriptor expiry at consumption;
  no raw public observation or commit token can bypass fresh authority or
  successful registered dispatch.
- 2026-08-25 -- Removed caller-constructible async membership and continuity
  authority. A framework-owned validation sink accepts at most one atomic
  snapshot from the host membership/current-registry port and independently
  matches its stream and full event contracts to the Task 2 authorized
  descriptor. A sequence machine is permanently bound to that exact
  subscription, stream, and signed authoritative baseline, with no public raw
  baseline input; cross-scope envelopes and recovery transcripts are rejected
  before position observation and cannot mutate current position, degradation
  state, or observed high-water.
- 2026-08-25 -- Made active subscription membership and registered stream
  identity prerequisites for constructing an asynchronous envelope, so an
  invalid or cross-subscription message cannot reach sequence observation or
  dispatch. Protocol-v1 decoding applies raw byte, nesting, entry, string, and
  canonical payload limits; accepted productive payloads are closed to the
  registered fresh render, exact typed browser-event contract and target, or
  declared presentation-signal schema. HTML, snapshots, actions, effects, and
  arbitrary operation names have no representable accepted form.
- 2026-08-25 -- Bound upload cleanup observability to closed identifier-free
  age/volume/outcome/retry/orphan values and made observers non-authoritative.
  Cleanup leases bind exact ledger revision while opaque handle, lease, scope,
  path, principal, grant, and raw-error values remain outside metric labels.
- 2026-08-25 -- Bound subscription descriptors to full registered event
  contracts and the mounted component-contract digest. Issue, connect, and
  renewal re-resolve current registry authority, and separate zeroizing
  credentials bind the exact descriptor, subscription scope, exclusive expiry,
  and Connect or Renew operation. The host continuity authority, never a public
  issue request, supplies the signed baseline. Host credential providers mint
  unique unpredictable bearers. After descriptor, registry, scope, expiry,
  signing, and current-policy checks, one distributed/restart-safe provider
  transaction consumes Connect and persists Renew or consumes Renew and persists
  Connect. Provider failure leaves the predecessor valid. If rotation commits
  but the response is lost, predecessor replay fails closed and recovery requires
  a freshly issued subscription. Trusted registration enforces the full
  canonical claims budget; mount-topic segments reject empty, traversal,
  encoded, and multi-segment forms; all credential-bearing debug surfaces are
  bounded and redacted.
- 2026-08-24 -- Closed cross-site WebSocket hijacking by requiring strict
  pre-upgrade `Origin` validation for cookie-authorized transports. Explicit
  cross-origin use requires a non-wildcard allowlist plus separate credential
  policy. Multiplexing handles remain non-authority; a subscription descriptor
  is a signed non-secret integrity authority memo, never a transport credential
  or substitute for current connect and renewal authorization.
- 2026-08-23 -- Separated Iteration 004 upload handles from secret transfer
  grants and signed subscription descriptions from any required transport
  credentials. Every operation reauthorizes current scope, and aggregate
  upload/stream limits plus secret-sentinel tests are required across control,
  provider, browser, observability, and cleanup paths.
- 2026-08-21 -- Replaced the public boolean promotion attestation with typed
  trusted-request check dispositions and dev-only harness construction.
  Explicitly kept host truth and middleware ordering as adapter/integration
  responsibilities rather than pretending an engine marker creates authority.
- 2026-08-21 -- Assigned the trusted Live request context capability and
  hostile host-adapter conformance suite to iteration 002. Actual Suprnova
  origin/CSRF/session/principal/tenant/proxy middleware construction remains an
  atomic-integration responsibility.
- 2026-08-21 -- Recorded the exact iteration 001 hostile-input and trusted-input
  boundaries, closed telemetry/redaction rules, parser fuzz coverage, and the
  session/auth/tenant/HTTP/DOM integrations that remain explicitly unclaimed.
- 2026-08-21 -- Security is an independent domain rather than an annotation on
  the wire spec. Existing Suprnova auth integration must be verified, not
  assumed.
- 2026-08-21 -- Signed snapshots provide integrity only; rejected treating them
  as encryption or current authorization proof.
- 2026-08-21 -- Public seed snapshots are replayable public mount state by
  design. Promotion creates only a new scoped instance and is authenticated,
  authorized, atomic, rate-limited, and storage-bounded.
