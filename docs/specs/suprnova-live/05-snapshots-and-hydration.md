# Suprnova Live -- 05 Snapshots and Hydration

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the public seed snapshot and instanced signed snapshot as
versioned, client-carried descriptions of one Live island, promotion from seed
to instance, the Live instance ledger contract, and deterministic dehydration
and hydration of component state. It depends on component lifecycle and state
categories and feeds the wire protocol, security checks, action execution, and
browser recovery. It does not own HTTP transport, authorization policy, DOM
output application, or deployment-tier provider selection.

## Capabilities

### Versioned snapshot schemas

Every mounted island shall receive either an instanced snapshot or a public seed
snapshot with an explicit form and schema version. An instanced snapshot shall
carry component and instance identity, state, lifecycle memo, revision, issuance
metadata, and integrity proof. A public seed shall carry the component/build
contract, route and island-slot identity, public mount parameters and state,
signing-key identity, bounded issue age, optional advisory dependency
generations, and integrity proof without carrying a reusable Live instance ID.
The schemas shall distinguish protocol evolution from application
component-state evolution.

Acceptance criteria:
- Required fields and canonical encoding are specified independently of any
  serializer's incidental defaults.
- Unknown required versions are rejected; optional extensions are ignored only
  under an explicit compatibility rule.
- Component, route, slot, instance where present, and identity binding cannot be
  substituted without invalidating the applicable form.
- Instanced revision and issuance metadata support stale, replay, and expiration
  checks; seed issue age and build compatibility bound reusable public markup.
- Cache retention and external shared-cache policy account for the seed
  acceptance window so an origin does not knowingly serve already
  unpromotable interactive markup; a previously opened document may still age
  into normal fresh-render recovery.
- A public seed contains no principal-bound, tenant-private, transient, secret,
  or already-instanced state.
- Snapshot contents remain inspectable browser data and are never described as
  secret merely because they are signed.

UX flow:
1. Server mounts or rerenders an island -> it emits the eligible seed or
   instanced form compatible with the rendered component contract.
2. Runtime presents an unsupported snapshot -> no action executes and the
   island follows the fresh-render recovery path.

### Public seed promotion

A cache-safe public island may remain inside immutable Complete representation
bytes by carrying a signed seed instead of a per-recipient instance. On the
first Live action, the runtime shall propose a cryptographically random
at-least-128-bit instance nonce together with the seed; the server shall treat
the nonce as untrusted identity input and atomically promote the verified seed
into a new scoped Live instance before accepting the action.

Acceptance criteria:
- Promotion verifies seed integrity, form/schema and component/build
  compatibility, bounded issue age, route and slot binding, request
  authenticity, current authorization, size/rate policy, and parameter schema.
- Instance creation is atomic and scoped to the current principal, session,
  tenant, or supported anonymous browser context where applicable; a nonce is
  never authorization evidence.
- Replaying a public seed may create only a new independently scoped instance
  under promotion limits and cannot join or replace an existing instance.
- Advisory dependency generations do not make unchanged cache coherence a
  mandatory promotion precondition; actions still reload and reauthorize
  authoritative data under their normal contracts.
- A component may declare `refresh_on_promote`; the server then reloads current
  authoritative component data before the first action and either reconciles
  allowed proposals or returns refresh-required without executing the action.
- Promotion and abandoned-instance retention are bounded so a reusable public
  seed cannot create unbounded ledger storage.

UX flow:
1. Application user first acts on a public cached island -> the same request
   promotes its seed and, when permitted, executes against the new instance.
2. Promotion is incompatible, limited, or requires unreconcilable refresh -> no
   action runs and the island obtains a fresh authorized rendering.

### Deterministic dehydration

Dehydration shall convert allowed component state and lifecycle metadata into a
bounded canonical representation, excluding server-only, computed, and secret
values. It shall run only after a successful lifecycle outcome eligible to be
published.

Acceptance criteria:
- Field order, numeric representation, null handling, and tagged types are
  deterministic for signing and fixtures.
- Every included field is permitted by the component-state schema.
- Server-only values are replaced by no value or an approved opaque reference,
  never serialized accidentally.
- Dehydration failure prevents snapshot issuance and successful response
  publication.
- Limits apply before unbounded allocation or signing work.
- Type-codec metadata is sufficient for lossless supported Rust round trips.

UX flow:
1. Component finishes a valid render -> dehydration emits bounded canonical
   state and memo.
2. A field cannot be safely represented -> the response fails with a developer
   diagnostic and the prior accepted browser state remains when possible.

### Verified hydration

Hydration shall reconstruct only the registered component type after protocol,
integrity, identity, size, and compatibility checks have passed. It shall not
call arbitrary constructors or materialize types selected by the browser.

Acceptance criteria:
- Verification precedes expensive or side-effectful hydration.
- The registered component schema controls field names and codecs.
- Missing, duplicate, unknown, or malformed required fields fail predictably.
- Hydration performs no domain writes, external calls, or authorization by
  deserialization side effect.
- Lifecycle hooks receive a clearly marked reconstructed component.
- Fuzzed or hostile snapshots cannot panic or cause unbounded recursion.

UX flow:
1. Valid action request arrives -> verified hydration reconstructs the component
   for the current request.
2. Hydration fails -> no action runs and the protocol returns a classified
   recovery response without echoing sensitive internals.

### Revision, instance-ledger, and freshness semantics

Instanced snapshots shall carry a monotonic island revision and bounded validity
information. An expiring Live instance ledger shall atomically arbitrate the
expected base revision, successor revision, idempotency identity, and accepted
outcome metadata without storing a persistent component object. Provider choice
may use process memory, the application database, or a compatible networked
key/value cache according to the deployment-tier contract.

Acceptance criteria:
- At most one committed Live outcome may be accepted for an island base
  revision; Live does not guarantee exactly-once Rust method invocation or
  external side effects.
- A provider that cannot transactionally couple its claim to domain effects
  claims the successor revision first. An attempt that fails without committing
  an outcome leaves that revision consumed and requires fresh-render recovery
  rather than replay.
- When the instance ledger and domain effects share one database transaction, a
  rollback may roll back both claim and effects; an idempotent retry may invoke
  the Rust method again.
- Every committed accepted outcome, including read-only, validation, and
  no-render outcomes, has explicit successor-revision behavior.
- An older snapshot cannot claim or overwrite a newer accepted or consumed
  island revision.
- Expiration policy distinguishes clock skew, deployment compatibility, and
  intentional maximum lifetime.
- Domain optimistic locking remains a separate application concern and can
  produce its own conflict outcome.
- Missing or expired instance-ledger state never reconstructs authority from a
  browser snapshot; it requires fresh rendering.

UX flow:
1. Runtime sends the current revision -> the ledger atomically claims or rejects
   its successor before incompatible action effects.
2. Browser state is obsolete -> stale HTML is not applied and the runtime
   obtains current authorized island state.

### Snapshot refresh and remount

The framework shall define recovery when a snapshot is validly rejected due to
expiration, compatibility, deployment, or unreconcilable revision. Recovery
shall obtain fresh server-authoritative HTML and state instead of mutating or
silently accepting the rejected payload.

Acceptance criteria:
- Rejection responses distinguish retryable transport failure from required
  refresh or remount.
- Fresh rendering uses the current route, identity, parameters, and
  authorization.
- Missing, expired, or consumed instance-ledger state is classified explicitly
  and cannot be recreated from the rejected instanced snapshot.
- Unsaved browser input is not silently reported as saved; applications may
  offer explicit restoration where safe.
- A controlled island replacement is permitted as recovery but is not the
  normal morph path.
- Repeated recovery failure terminates rather than loops indefinitely.

UX flow:
1. Runtime receives a refresh-required response -> it exposes recovery state and
   requests a fresh authorized island rendering.
2. Fresh rendering succeeds or fails -> the island resumes with current state
   or presents an actionable error without replaying the rejected action.

## Iteration 001 implementation profile

The checked v1 profile is implemented in this repository's internal
`suprnova-live` crate and documented in
[`snapshot-v1.md`](../../implementation/snapshot-v1.md). The signed envelope is
one canonical JSON object with exactly `body` and `signature`; `key_id` is
inside the signed body. The body uses `form = seed|instance` and
`schema_version = 1`, while its component contract independently carries
state, memo, and mount schema versions.

Canonical input is bounded before trusted use and rejects duplicate keys,
trailing data, invalid UTF-8, excessive bytes/depth/entries/strings, and JSON
numbers outside the finite interoperable IEEE-754 profile. Counters that can
exceed JavaScript's exact integer range use canonical decimal strings. Root
keys are at least 32 bytes; HKDF-SHA-256 derives separate seed-v1 and
instance-v1 keys, and HMAC-SHA-256 authenticates canonical body bytes.

Registered field metadata distinguishes public, locked, server-only, computed,
secret, and transient exposure. Bounded deterministic dehydration feeds the
canonical serializer; only verified seed or instance capability types expose
typed hydration. Seed promotion uses trusted adapter attestations, server-side
instance randomness, exact retry identity, and independently bounded rate,
outstanding, route/component, reservation, rate-bucket, and abandoned-retention
state.

`LiveInstanceLedger` is an async provider contract. Iteration 001 ships the
complete single-process memory reference with expected-revision claims,
provider-bound single-use claim tokens, duplicate outcome lookup, consumed and
expiry semantics, and bounded metadata. It does not store component state.
Database-coupled and daemon-backed implementations remain later provider work
and must preserve the same behavioral contract.

## Acceptance criteria

- Seed and instanced schemas, canonical encoding, promotion, and versioning are
  explicit.
- Only permitted state enters a snapshot and only registered schemas hydrate.
- Verification precedes hydration and action execution.
- An expiring instance ledger enforces one committed outcome per base revision
  without becoming server-resident component state or domain concurrency.
- Rejection has a bounded fresh-render recovery path.

## Decisions and revisions

- 2026-08-21 -- Recorded the implemented v1 profile: exact seed/instance forms,
  canonical number/counter rules, purpose-separated HKDF/HMAC keys, verified
  hydration capabilities, bounded promotion, and the complete Tier 0 memory
  ledger contract.
- 2026-08-21 -- Snapshots are signed and visible, not encrypted state or
  authorization proofs.
- 2026-08-21 -- Snapshot reconstruction is the default state model; rejected
  sticky server component memory.
- 2026-08-21 -- Added reusable public seed snapshots promoted on first action.
  Seed generations are advisory by default; `refresh_on_promote` opts a
  component into authoritative refresh before its first action.
- 2026-08-21 -- Added a tier-provider Live instance ledger. The guarantee is at
  most one committed outcome per base revision, not at-most-once method
  invocation or exactly-once external effects.
