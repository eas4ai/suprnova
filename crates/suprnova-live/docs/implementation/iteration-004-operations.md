# Iteration 004 operations

This guide records the artifacts, controls, evidence, and integration boundaries
that Iteration 004 shipped before the workspace cutover. The imported commands
remain an operator and conformance reference inside `crates/suprnova-live/`, not
a production Suprnova deployment guide.

## Artifacts

`rtk npm --prefix browser run build` creates a deterministic manifest-schema-v2
asset set. The manifest records engine version `0.1.0`, runtime contract version
1, Live protocol versions 1 and 2, snapshot version 1, exact bytes, SHA-256, SRI,
content type, script kind, preload relation, compatibility range, and immutable
cache policy.

| Trusted role       | File                                | Capability   | Selection                    |
| ------------------ | ----------------------------------- | ------------ | ---------------------------- |
| `core-esm`         | `suprnova-live.esm.js`              | `core@1`     | Required module runtime      |
| `core-classic`     | `suprnova-live.classic.js`          | `core@1`     | Required classic alternative |
| `stimulus-esm`     | `suprnova-live.stimulus.esm.js`     | `stimulus@1` | Optional module bridge       |
| `stimulus-classic` | `suprnova-live.stimulus.classic.js` | `stimulus@1` | Optional classic bridge      |
| `uploads-esm`      | `suprnova-live.uploads.esm.js`      | `uploads@1`  | Optional upload feature      |
| `uploads-classic`  | `suprnova-live.uploads.classic.js`  | `uploads@1`  | Optional upload feature      |
| `async-esm`        | `suprnova-live.async.esm.js`        | `async@1`    | Optional async feature       |
| `async-classic`    | `suprnova-live.async.classic.js`    | `async@1`    | Optional async feature       |

Choose ESM or classic for a document, not both. Trusted checked render metadata
selects optional roles; element attributes cannot supply an artifact URL. An
optional artifact registers with the singleton core lifecycle and never starts
a second runtime. A bundler can use the equivalent `@suprnova/live/runtime`,
`/stimulus`, `/uploads`, and `/async` exports. Uploads and async may be selected
independently.

The Stimulus artifact contains only Suprnova's bridge and continuity logic. It
imports or bundles no Stimulus package and requires an application-supplied
compatible `Application`. The uploads and async artifacts do not duplicate the
core or Idiomorph. Idiomorph 0.7.4 is bundled only in core. Production artifacts
contain no source maps, `eval`, dynamic function construction, inline handlers,
or server-returned script.

Serve the manifest content type and SRI with `public,
max-age=31536000, immutable`; modules use `modulepreload` and classic scripts use
`preload`. The runtime works with a strict `script-src 'self'` policy. A host may
nonce or hash its own bootstrap, but feature selection, connection endpoints,
and provider origins remain trusted configuration rather than DOM input.

Iteration 004's cross-language fixture corpus is `fixtures/v4/`. Upload protocol
v1 and async envelope/subscription protocol v1 are independent of Live update
protocols v1/v2. Feature registry ABI `suprnova.live.features.v1` registers
`uploads@1` and `async@1` against compatible core `>=0.1.0 <0.2.0`. These version
facts are checked, not inferred from package filenames.

## Limits

The engine rejects unbounded configuration before starting external work. The
reference upload profile selects 16 files per field, 128 pending uploads per
scope, 64 MiB per file, 256 MiB aggregate, 256 KiB chunks, 4,096 chunks per file,
8 MiB in-flight body bytes, eight concurrent transfers, 64 creations per
60-second window, 16 retries, a 24-hour temporary age, 30-second application
validation, 120-second scanning, 1 GiB quarantine storage, 256-item cleanup
batches, and 64 retained idempotency outcomes. Hosts may configure lower values;
engine ceilings remain enforced.

The browser upload manager defaults to 256 KiB chunks and four active transfers.
Its public ceilings are 64 files and handles per document, 16 active transfers,
4 MiB per chunk, 4 MiB queued bytes, and 4,096 code units for a secret grant.
The default `FetchUploadTransport` accepts at most 16 KiB of response JSON and
sends grants only in its upload authorization header.

Async subscription metadata permits 16 target scopes. The internal Rust
metadata type can represent fanout up to 1,024, but browser authorization and
registered-event admission reject values above 256. The effective end-to-end
event fanout ceiling is 256, further bounded by signed registration and
deployment policy. The independent replay transcript limit is 1,024 envelopes;
it is not event fanout. A component permits 32 subscriptions, with 32 topics and
64 event names per subscription, across the two closed transport modes.
Descriptors live at most 300 seconds, fallback polling is 1-300 seconds with at
most 100 percent configured jitter, and reconnect attempts are capped at 16.
Canonical envelopes are 64 KiB with 32 KiB payloads. The server document queue
is 64 events/256 KiB and owns at most 128 logical memberships. The browser
connection pool applies its own 256-membership ceiling, one physical transport
per compatible document key, eight concurrent WebSocket handshakes per origin,
and one queued plus one in-flight refresh per island.

Creation, chunk, status, completion, cancellation, reacquisition, validation,
finalization, cleanup, descriptor issue, credential rotation, transport create,
membership change, replay, and reconnect all have explicit request, byte,
deadline, concurrency, generation, and retirement controls. Exhaustion returns a
typed bounded failure or degraded freshness; it never removes a limit or grows a
queue because a peer is slow.

## Observability

Rust telemetry and the browser test ports expose low-cardinality operation,
lifecycle, result, and resource counts. Upload evidence includes admitted calls,
states, transitions, validation/scanner/finalizer/compensation/reconciliation
counts, active transfers, permits, queue depth/bytes, chunk-buffer ownership,
files, timers, cleanup obligations, and safe error kinds. Async evidence includes
logical memberships, physical SSE/WebSocket connections, authorization attempts,
open sockets/timers, queue events/bytes, pressure causes, refresh counts,
reconnect activity, and typed close/degraded outcomes.

No observer, span, metric, diagnostic, failure message, or benchmark record may
contain a transfer grant, transport credential, descriptor body, cookie, bearer
token, client file path, uploaded bytes, raw handle, topic, event payload,
authorization memo, island HTML, snapshot, or provider signature. Public browser
upload-resource observers use bounded document-local numeric slots for stable
per-transfer accounting; these slots are non-identifying and carry no upload
authority. Diagnostics are closed, capped at 256 records, and redact hostile
values before retention.

Reference-host inspection and compiled fault schedules are test controls. The
host refuses arbitrary request-selected fault injection and reports only bounded
identifier-free counters. Shutdown waits for sockets, files, timers, uploads,
memberships, and physical transports to drain; a failed drain remains a test
failure rather than disappearing from evidence.

## Benchmarks

Ordinary gates run deterministic reduced proofs. Qualified release evidence must
come from the pinned dedicated environments: S1 for Rust/server control paths and
B1 for Chromium/browser paths. The checked upload and async baselines currently
contain only `exploratoryReference`; `qualifiedBaseline` is `null`. Local results
are therefore useful unqualified evidence, never release qualification. Release
evaluation fails closed when the qualified baseline is absent, the candidate is
unqualified, the environment/artifact binding differs, or a hard cap/regression
fails.

`U4/16` transfers four concurrent 16 MiB files in 256 KiB chunks. Browser
framework ownership is at most two configured chunk buffers per transfer plus
256 KiB manager overhead, with progress application at most 16 ms p95 on B1.
Server ownership is at most two buffers per transfer plus 512 KiB manager
overhead, with control-plane work at most 2 ms p95 on S1. Body I/O, provider work,
scanning, and application validation are excluded from the control latency but
remain represented by explicit counters.

`E100/1K` uses 100 memberships over one physical document transport and delivers
1,000 ordered 1 KiB presentation events over ten seconds with 100 refresh
invalidations. It caps dispatch at 8 ms p95 on B1, retained framework memory at
8 KiB per active subscription, the document queue at 64 events/256 KiB, and
refreshes at one queued plus one in flight per island. Checked local evidence has
reported approximately 8.8 KiB retained per subscription and is explicitly
unqualified; it is not a passing B1 memory result.

`R100` simultaneously loses continuity for the same 100 memberships and proves
recovery with exactly one physical reconnect handshake. It has no invented
recovery-latency ceiling. After currentness it must return below 12 KiB retained
runtime per island. A separate 16-document run attempts 16 handshakes and proves
no more than eight concurrent handshakes per origin.

Artifact size uses Brotli quality 11 over deterministic production builds. Core
ESM/classic sizes are always reported and have no absolute cap. Stimulus has an
8 KiB per-format cap; uploads has a 20 KiB per-format cap. Async has no absolute
cap. Its append-only reviewed history currently records 21,396-byte ESM and
19,156-byte classic Brotli artifacts as reviewed correctness growth. Growth over
15 percent from the newest valid reviewed entry is an explicit review trigger,
not a total download limit. A candidate cannot overwrite or self-derive that
history.

Run the ordinary reduced evidence locally:

```sh
rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-upload-budget.sh
rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-async-budget.sh
```

Run full qualification only on dedicated pinned runners:

```sh
rtk env \
  SUPRNOVA_LIVE_BUDGET_PROFILE=qualified \
  SUPRNOVA_LIVE_S1_DEDICATED=1 \
  SUPRNOVA_LIVE_B1_DEDICATED=1 \
  scripts/run-upload-budget.sh
rtk env \
  SUPRNOVA_LIVE_BUDGET_PROFILE=qualified \
  SUPRNOVA_LIVE_B1_DEDICATED=1 \
  scripts/run-async-budget.sh
```

The complete ordinary project gate is:

```sh
rtk env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 scripts/gate.sh
```

`SUPRNOVA_LIVE_RELEASE=1` selects release branches in the gate but does not
create dedicated-runner attestations. A release runner must also provide the S1
and B1 environment controls required by the invoked scripts. Fabricating those
flags on a shared workstation does not make the resulting evidence qualified.
Clippy warnings are reviewed; the gate does not blanket-deny warnings.

## Reference-host boundary

`suprnova-live-test-support` ships the thin Rust reference host. Rust owns upload
authority, grants, ledger transitions, provider/quarantine I/O, validation,
scanning, finalization, compensation, cleanup, signed subscriptions, exact
WebSocket-origin validation, SSE/WebSocket membership, continuity, backpressure,
and fresh-render semantics. It validates and serves exact production browser
artifacts from the asset manifest.

The Node host owns only static scenario HTML, CSS, JavaScript drivers, and browser
test orchestration. It may reverse proxy those static scenarios through the Rust
host; it never implements a second upload or async state machine. The direct
provider bridge and `DirectProviderConformanceAdapter` emulate the constrained
provider boundary. Compiled fault schedules, inspection routes, deterministic
barriers, and benchmark routes exist only for conformance and are not production
administration APIs.

Build artifacts and run the reference-host suite with:

```sh
rtk npm --prefix browser run test:host
```

For manual conformance diagnosis, after a browser build:

```sh
rtk npm --prefix browser run host:iteration-004
```

These commands prove that the Rust engine drives the production browser
artifacts. They do not prove a cloud storage vendor, production broadcaster, CDN,
or Suprnova application deployment.

## Suprnova integration boundary

At Iteration 004 completion, development remained in the standalone checkout
and did not edit or register inside Suprnova. Iteration 005 has since imported
that committed history, product code, normative specifications, and checker
together under `crates/suprnova-live/`; the former checkout is immutable
historical provenance rather than a parallel maintained authority.

Iteration 004's durable test-tool classification remains exact: The Rust
reference host, Node static host, direct-provider bridge, fault controls, and
benchmark harnesses are conformance-only test tools, not production administration APIs.
They are neither Suprnova application integration nor vendor integration.

Suprnova application integration owns routes, authentication, session,
configuration, provider, scanner, storage, and broadcast wiring. Iteration 005
must implement and prove that ownership through framework tests; importing the
host-neutral reference machinery does not satisfy it. The integration must prove
real router and middleware registration,
trusted request context, authentication, authorization, session/CSRF/tenant
mapping, configuration, asset roles, application validation, database adapters,
storage and provider clients, scanner service, finalizer/domain transaction
wiring, cleanup scheduling, broadcaster/event-source adapters, and operational
tracing. The engine remains behind `suprnova::live`; applications do not depend
on this internal crate directly.

That framework adapter must preserve the shipped protocols and conformance
suites.
It must not copy the reference host into production, expose its example
reacquisition or fault routes as framework routes, make Node a semantic server,
or weaken exact origin, secret, revision, continuity, cleanup, and resource
bounds to match a vendor convenience API.
