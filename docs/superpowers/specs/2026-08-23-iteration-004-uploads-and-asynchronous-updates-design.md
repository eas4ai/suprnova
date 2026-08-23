# Iteration 004 Uploads and Asynchronous Updates Design

Status: Approved design
Approved: 2026-08-23

## Goal

Iteration 004 makes file uploads and asynchronous updates complete parts of the
standalone Suprnova Live engine. Application developers can bind native file
controls, transfer and validate files, finalize opaque upload handles through
ordinary Live actions, subscribe to typed server events, and use bounded polling
or push-driven refresh without introducing a client renderer or a second source
of component authority.

The iteration implements specifications 08 and 14 in full and extends the
browser, checker, protocol, testing, security, and observability contracts they
touch. It remains in the dedicated `suprnova-live` development workspace. The
reference host proves host-neutral transport behavior; it does not claim that
the active Suprnova checkout currently registers storage, broadcast, router, or
public facade adapters.

## Chosen approach

Uploads and asynchronous updates share a small bounded-resource foundation but
remain distinct protocols. The shared foundation owns lifecycle cancellation,
bounded queues, backpressure, document and island ownership, diagnostics,
observability, and entry into the existing island scheduler. It does not become
a generic bidirectional RPC layer.

The upload protocol owns selected browser files, temporary identity, transfer
grants, chunks, verification, quarantine, finalization, expiry, and cleanup. The
asynchronous-event protocol owns subscription descriptors, SSE and WebSocket
delivery, typed envelopes, sequence continuity, replay evidence, degraded
freshness, polling fallback, and presentation-only signal updates.

Completely separate implementations were rejected because they would duplicate
the same lifecycle, queue, cancellation, and diagnostics machinery. A generic
socket/RPC abstraction was rejected because it would erase the security and
authority differences between transferring untrusted bytes and receiving typed
notifications.

## Architecture

### Shared bounded-resource foundation

One document runtime owns upload and stream managers. Each operation remains
owned by a specific island, directive, and stable DOM identity and enters that
island's existing scheduler whenever authoritative Live work is required. The
managers share:

- structured task ownership and idempotent retirement;
- per-document, per-island, and per-origin concurrency accounting;
- byte, item, time, retry, and diagnostic bounds;
- visibility, offline, page lifecycle, bfcache, and navigation handling;
- redacted diagnostics and bounded telemetry;
- cancel, pause, resume, backoff, and shutdown primitives;
- deterministic clocks, randomness, transports, and stores for testing.

The shared layer carries no application payload schema, does not dispatch
actions by arbitrary name, and cannot apply HTML, snapshots, or component state.

### Optional production artifacts

The already-built core runtime occupies 46,057 bytes Brotli of its 46,080-byte
architecture cap, so uploads and asynchronous updates do not join the universal
bundle. They ship as two optional feature pairs:

- `suprnova-live.uploads.esm.js` and
  `suprnova-live.uploads.classic.js`;
- `suprnova-live.async.esm.js` and
  `suprnova-live.async.classic.js`.

The existing core ESM/classic pair remains at or below 45 KiB Brotli. Each
feature build includes the shared bounded-resource source it needs and registers
through one checked core extension surface; it does not create another runtime
instance or duplicate document listeners. A page using both features may load
both optional pairs without conflicting ownership.

The typed asset manifest records every artifact's role, module/classic format,
content hash, integrity, byte sizes, protocol/capability versions, and compatible
core range. Trusted server rendering declares required feature roles from
checked component/view metadata. Core startup resolves only those roles through
the manifest; element attributes cannot supply an artifact URL or module name.
ESM and classic startup load the corresponding variant deterministically,
deduplicate concurrent requests, honor CSP, and expose initial SSR content while
an unavailable/incompatible feature fails only its dependent directives.

### Upload control and data planes

The upload control plane creates an opaque upload handle and a separate
short-lived transfer grant. The handle identifies a temporary upload to trusted
server code but grants no authority by itself. A handle may appear as a typed
value in signed component state and normal action proposals. It is bounded,
expiring, principal/session-, tenant-, component-, field-, and policy-scoped,
and reauthorized whenever used.

The transfer grant authorizes only the declared create/status/chunk/complete/
cancel operations for one upload under explicit limits. It is secret-bearing and
must never enter component snapshots, rendered HTML, URLs, logs, traces,
diagnostics, browser history, or developer inspection output. It stays in the
runtime's current-document memory and is retired on completion, cancellation,
expiry, or terminal failure.

The data plane supports two provider-neutral modes:

1. **Reverse-proxy/file transport** streams bounded chunks through the Live host
   into a quarantined temporary store. This is the standalone reference
   implementation and works without an object-storage daemon.
2. **Direct-to-storage transport** asks a provider for bounded transfer
   instructions, sends bytes directly under the provider's constrained grant,
   and reports completion for authoritative verification. The capability and
   conformance contract are implemented now; concrete S3-compatible or vendor
   adapters wait for Suprnova integration.

Both modes expose identical handle authority, lifecycle, verification,
finalization, cleanup, progress, cancellation, and application-facing results.
Provider choice may change performance and deployment requirements but cannot
weaken semantics.

### Asynchronous update plane

The server renders a signed, bounded subscription descriptor for each declared
stream. It contains registered stream identity, protocol version, endpoint or
host capability, topics, allowed typed events, scope, authorization context
memo, authoritative baseline epoch/sequence, expiry, and reconnect policy. It
contains no arbitrary JavaScript, attacker-selected action, trusted HTML,
snapshot, or domain authority.

SSE and WebSocket transports consume the same versioned event-envelope schema.
Transport differences remain below the envelope and continuity contracts.
Applications may also choose polling-only or push-only policy. In the default
hybrid policy, authorized push is primary, polling pauses while continuity is
proved, and bounded jittered polling resumes when continuity is uncertain.

A push envelope may schedule only:

- a registered authoritative refresh through the island scheduler;
- a registered typed browser event;
- a declared presentation-only local-signal update.

It may not invoke a mutating Live action automatically. Domain mutation occurs
in an ordinary authenticated server handler, queue consumer, application
service, or deliberate user-initiated Live action; that work may then publish an
invalidation. Authoritative HTML and successor snapshots always return through
the existing verified Live refresh/action response path.

### Ownership and lifecycle

Browser `File` objects, transfer grants, stream transports, replay positions,
and fallback timers are current-document resources. They are never dehydrated
into component snapshots or persisted by default in `localStorage`,
`sessionStorage`, or IndexedDB.

Compatible morphs preserve an active upload or subscription only when its
stable keyed owner, directive contract, and island identity survive. Removing
or replacing the owner cancels or retires the resource according to declared
policy. Navigation and document retirement stop future browser application and
perform best-effort cancellation without claiming rollback of server work.

Guaranteed upload resume is limited to the current document/connection
lifecycle. After reload or process restart, an application may expose an
ordinary authenticated route that securely reacquires a handle and a new
transfer grant. Without that explicit application path, the upload expires and
server cleanup reclaims it.

## State and data flow

### Upload lifecycle

The authoritative temporary-upload state machine is:

```text
created -> queued -> transferring -> verifying -> ready -> finalizing -> finalized
   |          |           |              |          |            |
   +----------+-----------+--------------+----------+------------+
              -> rejected | canceled | expired | failed
```

Every transition carries a monotonically increasing upload revision and is
performed through a conditional transition against the expected revision.
Duplicate chunks, completion notices, cancellations, status requests, and
finalization attempts return an idempotent existing result or a typed conflict;
they never move state backward or manufacture a second durable outcome.

The browser flow is:

1. `live:upload` validates immediate client-known count and size constraints,
   preserves native file-input security, and requests temporary identity.
2. The server returns an opaque handle and a separate transfer grant through a
   confidential control response.
3. The document upload manager queues bounded chunks, reports truthful progress,
   and honors server/provider backpressure.
4. Transfer completion moves the upload to verification, where authoritative
   size, integrity, type/content policy, metadata parsing, and optional scanning
   run against quarantined bytes.
5. A successful upload becomes `ready`; only the opaque handle becomes eligible
   for typed component proposal/action use.
6. A deliberate Live action reauthorizes and consumes the ready handle. It
   prepares durable storage, commits application database effects, and completes
   or compensates storage work under an explicit provider strategy.
7. Terminal or abandoned temporary state is reclaimed by idempotent observable
   cleanup even when no browser callback arrives.

Live does not pretend storage and a relational database form one atomic
transaction. Finalization exposes preparation, commit, compensation, retry, and
reconciliation results. An application action remains safe for method
reinvocation before commit and cannot claim exactly-once external side effects.

### Stream lifecycle

The browser stream state machine is:

```text
disconnected -> connecting -> current -> degraded -> reconnecting -> closed
                    |            ^          |             |
                    +------------+----------+-------------+
                         continuity proof or refresh
```

Each accepted event identifies the stream, subscription, schema, type, epoch,
and monotonic sequence. The descriptor's server-issued baseline binds initial
SSR state to the first required event; if the transport cannot replay from that
position, it refreshes before claiming current. Duplicates are ignored. A gap
stops presentation of the stream as current. The transport may become current
again only after it proves replay continuity from a trusted resume position or
after an authoritative island refresh establishes a new baseline.

In hybrid mode, polling is dormant while push continuity is proved. A disconnect,
gap, heartbeat failure, authorization uncertainty, or bounded replay failure
makes freshness degraded and activates jittered polling. Successful transport
reconnection alone does not claim currentness; continuity proof or refresh is
required. Poll and push invalidations coalesce through the same island scheduler
without overlapping unsafe refreshes.

## Browser authoring contract

Iteration 004 promotes four previously reserved directive families into the
closed grammar:

- `live:upload` declares an upload field and its registered cancel, retry, and
  remove controls;
- `live:progress` exposes scoped truthful upload progress and status;
- `live:poll` declares bounded refresh polling policy;
- `live:stream` declares a registered subscription and freshness policy.

Existing `live:on`, feedback, local signals, scheduler, preservation, and
navigation contracts remain authoritative. Directives carry registered names or
typed literal configuration, never executable expressions, arbitrary endpoints,
channel interpolation, or transfer secrets. The generated metadata, Askama
checker, TypeScript parser, and shared fixture corpus agree on every directive,
modifier, value kind, conflict, and production fallback.

Native file controls remain the selection authority. Live never assigns a file
path or synthesizes a browser `File`. Progress exposes queued, transferring,
verifying, ready, finalizing, finalized, interrupted, failed, canceled, and
expired distinctions where material. Cancel/retry/remove controls remain
keyboard operable and named; progress announcement is throttled to avoid
live-region noise.

Material stream freshness exposes connected/current, degraded, reconnecting,
polling fallback, denied, and closed states through semantic attributes and
feedback targets. Features for which connection state is immaterial do not
create unsolicited announcements.

## Protocol boundaries and compatibility

The existing Live action/morph protocol remains version 2 unless implementation
proves that its request or response semantics must change. Upload handles travel
through Live as typed opaque values; file bytes and transfer grants do not.

The upload control/data protocol and asynchronous event-envelope protocol each
have an independent explicit major version, bounded codecs, media/transport
rules, and compatibility fixtures. They are not folded into a generic Live
protocol version 3. Unknown major versions fail closed. Compatible additive
optional fields may evolve only under the documented minor-version rules.

The directive grammar and cross-language conformance corpus advance to version
4 for the promoted directive families. Runtime/host capability metadata
advertises supported upload modes, stream transports, and protocol versions so
mismatches produce deterministic bounded diagnostics rather than partial
activation.

## Security and failure policy

All browser metadata is untrusted. Count, per-file size, aggregate size, chunk
size, in-flight chunk count, document/island concurrency, creation rate,
temporary storage, verification time, scan time, retry count, and diagnostic
retention are bounded server-side. Original filenames are bounded display data,
never paths. MIME, extension, checksum, dimensions, and media metadata are
verified under bounded parsers appropriate to application policy.

Temporary bytes remain quarantined and non-public until authorized finalization.
Scanning supports asynchronous completion, timeout, explicit unavailable policy,
and safe rejection. Rejected, canceled, expired, failed, and abandoned content
cannot be finalized and enters idempotent cleanup. Cleanup exposes bounded age,
volume, outcome, retry, and orphan metrics without filenames, paths, grants, or
content.

SSE and WebSocket setup reauthenticate and authorize principal, tenant,
component, stream, and topic. Connections, subscriptions, messages, payload
bytes, heartbeat intervals, replay windows, fanout, server buffers, client
queues, reconnect attempts, and fallback polling are bounded. Slow consumers
are coalesced, degraded, or disconnected according to declared event semantics;
unbounded buffering is forbidden.

Malformed upload/stream values, invalid state transitions, revoked authority,
sequence gaps, unsupported versions, and provider failures return closed typed
dispositions. Secret-bearing grants and subscription tokens are redacted from
all normal failure output. Ordinary routes, Live actions, HTTP forms, and
navigation remain usable when upload or real-time enhancement is unavailable.

## Verification strategy

The standalone reference host exercises real chunked HTTP transfer, the
provider-neutral direct-storage contract, authorized SSE, authorized WebSocket,
polling fallback, and ordinary Live refresh using the exact production-mode
browser runtime artifacts:

- `browser/dist/suprnova-live.esm.js`;
- `browser/dist/suprnova-live.classic.js`;
- `browser/dist/suprnova-live.uploads.esm.js`;
- `browser/dist/suprnova-live.uploads.classic.js`;
- `browser/dist/suprnova-live.async.esm.js`;
- `browser/dist/suprnova-live.async.classic.js`;
- `browser/dist/suprnova-live.assets.json`.

Those are deterministic, minified, CSP-safe, versioned, hashed release-mode
files without production source maps. The host serves the exact artifacts rather
than TypeScript source, a development transform, or a test-only bundle. This
proves distributable browser behavior but does not claim deployment or
registration by the active Suprnova framework.

Rust unit, property, concurrency, and integration tests cover upload and stream
state machines, conditional revisions, idempotency, token/grant scope, provider
conformance, finalization compensation, cleanup, replay continuity, fallback
policy, backpressure, and redaction. Fuzz targets own all new untrusted codecs
and state-transition entry points.

The shared version-4 fixture corpus is consumed by Rust, the Askama checker, and
TypeScript for directive grammar, protocol versions, upload/event envelopes,
state transitions, capability negotiation, diagnostics, and compatibility.
Playwright runs the served production-mode artifacts in pinned Chromium,
Firefox, and WebKit and covers keyboard/focus, native file restrictions, morph
continuity, cancellation, interruption, current-document resume, expiry,
progress accessibility, SSE/WS/poll switching, gaps, duplicates, reconnect,
slow consumers, bfcache, navigation, CSP, offline behavior, and resource cleanup.

Adversarial suites force quota exhaustion, oversized chunks/messages, transfer
and finalization races, scan timeouts, cancel/finalize collisions, forged
handles, leaked-grant sentinels, revoked subscriptions, reconnect storms, replay
overflow, fanout pressure, and late delivery after retirement. Deterministic
barriers, clocks, transports, and provider faults replace correctness sleeps.

The existing architecture budget v1 remains binding and gains three canonical
workloads. `U4/16` transfers four concurrent 16 MiB files in 256 KiB chunks
through the loopback reference provider. `E100/1K` connects 100 subscribed
islands and delivers 1,000 ordered 1 KiB presentation events over ten seconds,
with ten percent of events producing coalescible refresh invalidations. `R100`
simultaneously removes continuity from those 100 subscriptions and exercises
jittered reconnect plus polling fallback.

Release-blocking limits are:

- the upload ESM and classic artifacts are each at most 20 KiB Brotli;
- the async ESM and classic artifacts are each at most 16 KiB Brotli;
- `U4/16` retains at most two configured chunk buffers per active transfer plus
  256 KiB of browser-manager overhead and two configured chunk buffers per
  active server transfer plus 512 KiB of server-manager overhead;
- `U4/16` progress application takes at most 16 ms p95 on `B1`, and upload
  control-plane framework overhead takes at most 2 ms p95 on `S1` outside body
  I/O, provider work, scanning, and application validation;
- `E100/1K` retains at most 8 KiB framework memory per active subscription,
  excluding native transport buffers, DOM, and the currently dispatched
  application payload; queued unapplied browser events are capped at 64 items
  and 256 KiB per document;
- `E100/1K` typed presentation dispatch takes at most 8 ms p95 on `B1`, and
  invalidations retain at most one queued plus one in-flight refresh per island;
- `R100` permits at most eight concurrent reconnect handshakes per origin,
  creates no synchronized polling burst, and stays within the existing 12 KiB
  per-island retained-runtime cap after returning to current state.

Larger application file or payload policies do not authorize unbounded
framework memory. No existing budget may be relaxed merely to fit the new
implementation; a proposed cap revision requires a separate dated rationale and
developer approval.

## Scope boundary

Iteration 004 does not modify or integrate the active Suprnova or Magnetar
worktrees. It does not provide concrete S3-compatible/vendor storage adapters,
Suprnova broadcast adapters, final public facade/CLI wiring, RenderCache,
component-library widgets, Tailwind/theme work, persistent browser upload
storage, streamed HTML, push-triggered mutation, or SPA navigation.

An application may provide its own authenticated upload-reacquisition route,
storage adapter, or broadcast adapter through the checked host-neutral traits.
Those extension points do not become claims that the corresponding Suprnova
integration exists. The dedicated development workspace remains authoritative
until separation materially blocks a coherent change.
