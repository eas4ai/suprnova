# Suprnova Live -- 07 Security and Trust Boundaries

Status: Normative design specification
Last revised: 2026-08-21

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
- Cross-origin use is denied unless an explicit supported deployment contract
  enables it.

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
- Logs redact state values, cookies, signatures, CSRF tokens, upload tokens, and
  sensitive arguments.
- Metrics use bounded labels and do not create attacker-controlled cardinality.
- Security failures retain correlation identifiers suitable for investigation.

UX flow:
1. Legitimate application user reaches a limit -> accessible feedback states
   whether and when retry is possible.
2. Hostile traffic exceeds policy -> requests are rejected cheaply and safely
   without exposing enforcement internals.

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

- 2026-08-21 -- Security is an independent domain rather than an annotation on
  the wire spec. Existing Suprnova auth integration must be verified, not
  assumed.
- 2026-08-21 -- Signed snapshots provide integrity only; rejected treating them
  as encryption or current authorization proof.
- 2026-08-21 -- Public seed snapshots are replayable public mount state by
  design. Promotion creates only a new scoped instance and is authenticated,
  authorized, atomic, rate-limited, and storage-bounded.
