# Trusted interaction spine threat model v1

## Assets and security goals

Iteration 001 protects snapshot integrity and binding, instance revision
authority, promotion storage bounds, protocol structural integrity,
cross-language compatibility, and diagnostic secrecy. It aims to ensure that
hostile bytes cannot select arbitrary Rust code, hydrate before verification,
cross a component/route/slot/scope binding, overwrite newer island authority,
join another promotion, allocate without configured bounds, or leak payloads
through normal errors and telemetry.

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
- the future request-authenticity/identity/authorization context after its
  middleware has independently succeeded.

A valid HMAC proves that signed canonical bytes were issued under one purpose
key. It proves neither secrecy, current authorization, domain freshness,
request authenticity, nor continued ledger authority.

## Threats and implemented controls

| Threat | Iteration 001 control |
|---|---|
| Field or form tampering | Canonical signed body; verify before typed decode/hydration |
| Key or purpose substitution | Key ID inside body; HKDF purpose/version separation |
| Replay or stale revision | Expiring instance ledger; expected-base CAS semantics; bounded duplicate metadata |
| Cross-component/route/slot/scope use | Trusted expectation binding after integrity verification |
| Public-seed storage/CPU exhaustion | Rate, outstanding, route/component, reservation, bucket, and abandoned-retention bounds; indexed counts; deadline queues with bounded per-admission cleanup |
| Nonce reuse or instance joining | Browser nonce is non-authoritative; exact retry tuple matching; server-generated instance IDs |
| Malformed/large/deep input | Pre-parse byte limit and duplicate-aware byte/depth/entry/string bounds |
| Arbitrary dispatch | Protocol parser returns validated names and values only; no registry lookup or method call occurs |
| Confused response application | Complete response validation; terminal redirects; morph-before-commit planner |
| XSS through effects | Effects remain registered names plus bounded data; no script evaluator exists |
| Secret/error leakage | Redacted snapshot/request debug paths; closed safe error and telemetry dimensions |
| Parser panic | Property tests, persisted regressions, and one nightly fuzz target per external parser/verifier |

The memory ledger's opaque claim token is bound to the issuing provider and can
commit once. Claim failure never rolls revision backward. Promotion verifies
integrity and trusted bindings before generating identity or creating ledger
state. Telemetry accepts only closed event/outcome/error enums plus an optional
fixed-width digest prefix, never raw component, route, key, identity, or
payload strings.

## Provider guarantees and failure model

Tier 0 is a complete behavioral provider, not a weaker correctness mode. Its
process memory is appropriate only for the single-process deployment profile,
but within that profile it enforces the full revision/promotion semantics.
Future database and daemon-backed providers must pass the same conformance
contract and add their topology-specific transaction, fencing, eviction, and
partition evidence.

The ledger guarantees at most one committed accepted Live outcome per base
revision. It does not guarantee one Rust method invocation, exactly-once
external effects, or rollback across an uncoupled provider. Provider, clock,
randomness, key, parse, verification, compatibility, and limit failures are
classified and fail before uncertain success publication.

## Explicitly deferred integrations

Iteration 002 owns the actual HTTP endpoint, content types/status, origin and
CSRF enforcement, cookie/session lifecycle, current principal and tenant
resolution, authorization, component/action registries, domain reload, and
Suprnova provider adapters. Iteration 003 owns CSP/runtime delivery, DOM
morphing, URL enforcement in the browser, registered effect execution, and
post-acceptance recovery. This repository's conformance model constrains those
integrations but does not claim they exist.

Consequently, using the internal parser or a verified snapshot by itself is
never sufficient authority to perform an application action.
