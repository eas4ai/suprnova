# Trusted interaction spine and component-kernel threat model

## Assets and security goals

Iterations 001 through 003 protect snapshot integrity and binding, instance
revision authority, promotion storage bounds, protocol structural integrity,
generated component dispatch, host-context admission, rendering boundaries,
endpoint response intent, cross-language compatibility, and diagnostic secrecy.
Hostile bytes must not select arbitrary Rust code, hydrate before verification,
cross a component/route/slot/scope binding, overwrite newer island authority,
join another promotion, forge child parameters, inject response authority,
allocate without configured bounds, or leak payloads through normal errors and
telemetry.

Snapshot contents and public seeds are not confidentiality assets. They are
browser-visible by design. Domain data, authorization, tenant isolation,
sessions, CSRF proofs, signing roots, and external effects remain protected by
their owning server integrations rather than by snapshot signatures.

## Trust boundaries

Untrusted inputs include every canonical byte string, signed envelope,
signature and embedded key ID, browser nonce, component/action/model identity,
argument, revision, correlation/idempotency value, response, redirect, error,
extension, and benchmark or fixture file read by a test.

Trusted inputs are deliberately narrow:

- registered component and state schemas;
- framework-supplied build, route, slot, and scope expectations;
- configured root keys and rotation windows;
- injected clocks and server-side random instance generation;
- validated provider configuration and opaque provider-issued claim tokens; and
- a `TrustedLiveRequestContext` after the host validator has matched complete
  current check facts, scope capabilities, and the immutable mount catalog.

A valid HMAC proves that signed canonical bytes were issued under one purpose
key. It proves neither secrecy, current authorization, domain freshness,
request authenticity, nor continued ledger authority.

## Host and endpoint threats

| Threat | Implemented control |
| --- | --- |
| Host omits or fabricates a prerequisite | Every origin, CSRF, session, principal, tenant, proxy, rate-limit, and middleware check has one current typed disposition |
| Context crosses request identity | Scope/session/principal/tenant fingerprints must match request-bound capabilities and mount requirements |
| Route, slot, component, or protocol drift | Startup catalog is proved against the immutable generated registry; endpoint rechecks catalog, request, media, and snapshot facts |
| Cached or malformed action transport | Explicit cache bypass, POST-only exact vendor media, whole-body bounds, and strict duplicate-aware parsing before dispatch |
| Context expires during work | Endpoint checks context before and after kernel dispatch and suppresses late publication |
| Kernel smuggles unsafe HTTP output | Complete response is parsed, outcome/correlation/successor-verified, canonically re-encoded, and bounded before typed HTTP intent exists |
| Authn/authz disclosure | Host concealment produces `404` with no kernel body or Live media type |

The host remains responsible for making the supplied facts truthful. A typed
`Passed` value is not magic authentication; it is a narrow integration boundary
whose production adapter must be covered by framework tests. The host-neutral
hostile-adapter suite proves that missing, expired, inconsistent, cross-route,
cross-principal, and cross-tenant facts cannot enter kernel execution.

## Component-kernel threats

| Threat | Implemented control |
| --- | --- |
| Arbitrary component or action dispatch | Generated stable metadata plus explicit immutable registry/action tables; no reflection, global inventory, or Rust path from browser text |
| Mass assignment | Registered typed model paths/codecs and category checks reject unknown, locked, server-only, session, computed, and secret targets before setters |
| Parent-to-child authority forgery | Purpose-separated `child-params-v1` signature bound to accepted parent revision, child key/contract, schema, value digest, and expiry |
| XSS through views | Askama default escaping, raw `safe` checker rejection, auditable `TrustedHtml`, branch-aware HTML checking, and engine-owned inert mount wrappers |
| Sticky state confusion | One fresh owned component per request, verified reconstruction, exactly-once teardown ownership, and no component object in the ledger |
| Partial publication | Complete render/dehydrate/sign/output validation precedes private mount authority or accepted action publication |
| External-effect replay assumptions | At-most-one accepted committed outcome per base revision only; action/outbox documentation rejects exactly-once claims |
| Child/parent rollback confusion | Pending child parameters are separately authorized and child failure recovers child-locally after accepted parent state |

## Snapshot and protocol threats

| Threat | Implemented control |
|---|---|
| Field or form tampering | Canonical signed body; verify before typed decode/hydration |
| Key or purpose substitution | Key ID inside body; HKDF purpose/version separation |
| Replay or stale revision | Expiring instance ledger; expected-base CAS semantics; bounded duplicate metadata |
| Cross-component/route/slot/scope use | Trusted expectation binding after integrity verification |
| Public-seed storage/CPU exhaustion | Rate, outstanding, route/component, reservation, bucket, and abandoned-retention bounds; indexed counts; deadline queues with bounded per-admission cleanup |
| Nonce reuse or instance joining | Browser nonce is non-authoritative; exact retry tuple matching; server-generated instance IDs |
| Malformed/large/deep input | Pre-parse byte limit and duplicate-aware byte/depth/entry/string bounds |
| Arbitrary dispatch | Protocol parser returns validated names and values; trusted catalog, immutable registry, and closed action/lifecycle tables resolve them |
| Confused response application | Complete response validation; terminal redirects; morph-before-commit planner |
| XSS through effects | Effects remain registered names plus bounded data; no script evaluator exists |
| Secret/error leakage | Redacted snapshot/request debug paths; closed safe error and telemetry dimensions |
| Parser panic | Property tests, persisted regressions, and one nightly fuzz target per external parser/verifier |

## Browser-runtime threats

| Threat | Implemented control |
|---|---|
| Executable directive or effect injection | Generated closed directive grammar; registered effects/calls with schemas and deadlines; no evaluator or dynamic module selection |
| Arbitrary action endpoint or cross-origin exfiltration | One bounded document config; same-origin default; exact boot-time origin allowlist; fixed credentials policy |
| Forged island ownership or seed authority | Closed inert metadata, snapshot public-view agreement, nested ownership checks, first-intent nonce, no browser signer |
| Response races and stale DOM publication | One bounded scheduler per island, revision/identity eligibility, shared ordering fixture, morph-before-metadata-commit |
| Morph XSS or structural escape | Bounded detached parse, exact compatible root, identity/control/teleport preflight, private pinned Idiomorph adapter |
| Local-state privilege confusion | Typed local-only signal graph; local values never dehydrate or authorize models/actions |
| Sensitive form/diagnostic retention | Newer-edit/IME-aware ephemeral continuity; closed diagnostic objects; no payload/exception retention |
| Listener, observer, timer, or controller leaks | One document resource ledger, idempotent suspend/resume/dispose, all-engine lifecycle/leak tests |
| False browser-support claim | Pinned Playwright conformance kept distinct from authenticated actual-product floor evidence |
| Runtime or dependency drift | Exact npm lock, generated-contract check, byte-identical build, manifest hashes/SRI, bundle/license gates |

The memory ledger's opaque claim token is bound to the issuing provider and can
accept once. Claim failure never rolls revision backward. Promotion verifies
integrity and trusted bindings before generating identity or creating ledger
state. Telemetry accepts only closed event/outcome/error enums plus an optional
fixed-width digest prefix, never raw component, route, key, identity, or
payload strings.

## Provider guarantees and failure model

Tier 0 is a complete behavioral provider, not a weaker correctness mode. Its
process memory is appropriate only for the single-process deployment profile,
but within that profile it enforces the full revision/promotion semantics.
Database and daemon-backed providers in their named deployment iteration must
pass the same conformance contract and add their topology-specific transaction,
fencing, eviction, and partition evidence.

The ledger guarantees at most one accepted committed outcome per base revision.
It does not guarantee exactly-once method invocation or external effects, or
rollback across an uncoupled provider. Provider, clock, randomness, key, parse,
verification, compatibility, and limit failures are classified and fail before
uncertain success publication.

## Framework integrations still in progress

Iteration 002 implements the host-neutral endpoint, trusted-context validator,
component/action registry, authorization/validation/transaction ports, and
conformance services. Actual Suprnova router/middleware/request/response,
origin/CSRF, cookie/session, principal/tenant, policy, database transaction,
outbox, and provider adapters remain Iteration 005 work until their framework
integration tests pass; colocation does not make them complete.

Iteration 003 implements CSP-compatible runtime delivery metadata, DOM
morphing, URL execution in the browser, registered effect execution,
scheduling, feedback, lifecycle, and post-acceptance recovery in the host-
neutral browser harness. It does not claim that Suprnova's router, response
pipeline, asset publisher, sessions, or middleware have registered those
pieces.

Consequently, using the internal parser or a verified snapshot by itself is
never sufficient authority to perform an application action.
