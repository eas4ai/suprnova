# Snapshot v1 implementation contract

Iteration 001 implements two distinct signed forms. Both are canonical JSON
objects containing exactly `body` and `signature`; the key ID is inside the
signed body so key selection cannot be substituted independently.

## Canonical and cryptographic profile

- Input is one bounded UTF-8 JSON value with duplicate keys, trailing data,
  excessive bytes, depth, entries, strings, and non-interoperable numbers
  rejected before trusted use.
- Canonical bytes follow the RFC 8785 JSON Canonicalization Scheme profile,
  including UTF-16 object-key order, ECMAScript finite-number serialization,
  and negative-zero normalization.
- Counters that may exceed JavaScript's exact integer range use validated
  canonical decimal strings.
- At-least-32-byte root keys derive separate 32-byte `seed-v1` and
  `instance-v1` keys with HKDF-SHA-256. HMAC-SHA-256 signs canonical body bytes;
  verification uses the RustCrypto constant-time API.
- Key records have explicit signing, verification, and retirement windows.
  Unknown, weak, inactive, retired, or malformed key material fails closed.

Snapshots are integrity protected, not encrypted. Every included value is
browser-visible and must be eligible for that exposure independently of its
signature.

## Seed body

The v1 seed body contains `form`, `schema_version`, the component name and
contract digest, independent state/memo/mount schema versions, build ID, route
digest, island slot, key ID, issue time, maximum age, public mount/state/memo,
advisory dependency generations, `refresh_on_promote`, and namespaced
extensions. It contains no scope, principal, tenant-private state, instance ID,
revision, secret, transient field, or reusable instance authority.

Verification binds the body to trusted framework expectations for component,
build, route, slot, schemas, current time, and key ring before returning
`VerifiedSeedV1`. Only that capability exposes typed hydration methods.

Promotion additionally requires a trusted adapter context, a bounded browser
nonce, promotion policy, a server-side random instance generator, and a
`LiveInstanceLedger`. The browser nonce is identity input only. Exact retries
may recover the same reservation; changed seed/scope/idempotency input cannot
join it. Rate buckets, outstanding instances, route/component counts,
reservations, and abandoned retention are independently bounded. Advisory
generations are memo rather than authority; `refresh_on_promote` is an explicit
component choice.

## Instanced body and revision authority

The v1 instance body contains the common component/build/route/slot/key fields
plus trusted scope fingerprint, server-generated instance ID, monotonic
revision, issue/expiry times, typed state/memo, and namespaced extensions.
Verification also binds the current trusted scope and returns
`VerifiedInstanceV1`; missing ledger authority cannot be reconstructed from the
browser-carried body.

`LiveInstanceLedger` stores bounded concurrency and idempotency metadata, never
a component object. The complete Tier 0 memory provider proves:

- one grant for an expected base revision and monotonic successor claims;
- exact duplicate observation of a compatible pending or accepted outcome;
- stale, mismatched, expired, and consumed authority rejection;
- provider-bound opaque one-use claim tokens;
- bounded accepted-outcome history, instance lifetime, and claim leases; and
- one committed accepted Live outcome per base revision, without claiming
  exactly-once Rust invocation or external effects.

Tier 0 claims the successor before uncoupled work. Abandonment or claim expiry
consumes authority and requires a fresh render; it is never repaired by rolling
the ledger revision backward.

## State schemas and codecs

`SnapshotSchemaSet` independently versions state, memo, and mount schemas.
Every field declares a codec, exposure category, and requiredness. Public,
locked, server-only, computed, secret, and transient categories remain
distinct. Only permitted fields dehydrate; unknown, missing, wrong-codec, or
wrong-exposure values fail. JSON, exact tagged signed/unsigned integers, bytes,
and the implemented tagged types round-trip without lossy coercion.

Dehydration first writes through a byte-bounded serializer, parses through the
same duplicate-aware canonical boundary, validates the registered schema, and
then canonicalizes. Verification and binding checks always precede hydration.
Normal `Debug` and error formatting redact bodies, signatures, and state.

## Failure and recovery boundary

Snapshot parsing produces closed error kinds for size/depth/entry violations,
duplicate fields, invalid envelope/form/schema/state, signature/key failures,
binding or compatibility mismatch, invalid time windows, expiry, and extension
violations. These errors execute no component action. The later HTTP adapter
maps them to the protocol's retain, refresh, remount, navigate, or stop
instruction.

Iteration 001 does not implement Suprnova session, CSRF, authorization, tenant,
HTTP middleware, component dispatch, domain reload, or DOM recovery. Those
trusted inputs are explicit adapter obligations for iterations 002 and 003.
