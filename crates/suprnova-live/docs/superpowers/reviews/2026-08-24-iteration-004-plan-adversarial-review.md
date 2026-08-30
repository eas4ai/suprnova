# Iteration 004 Implementation Plans Adversarial Review

**Reviewed:** 2026-08-24
**Scope:** `iterations/004.md` (amended 2026-08-24), the approved uploads and
asynchronous-updates design, and the four plans under `docs/superpowers/plans/`:
shared foundation (executed through `820cde2`), uploads, async updates,
integration/hardening.
**Worktree state at review:** `iteration-004-uploads-async` at `820cde2`, 22
commits ahead of `main`, foundation gate closed. Two files carry uncommitted
edits (`browser/e2e/bootstrap.spec.ts` race fix, `docs/implementation/local-reactivity.md`
prose) and `iterations/next/browser-enhancement-substrates-vnext.md` is
untracked. Nothing in this review depends on those.
**Verdict:** Not ready for Plans 2 and 3 as written. Four seams would let an
implementation pass every named check and still miss the contract; each is a
planning defect with a concrete remediation below, not new product scope. The
foundation as executed is sound and nothing here requires reverting it.

## Review question

Could an implementer follow Plans 2, 3, and 4, pass their listed commands, and
still fail the 37-item definition of done through a server/browser seam, a
port that is closed one capability short, a reference host that stops being
evidence, a workload the browser cannot physically run, or an authority claim
the grammar quietly contradicts?

Yes, on four counts (B1-B4). The remaining findings are important but
locally fixable.

## Blocking findings

| ID | Adversarial finding | Evidence | Locked remediation (proposed) |
| --- | --- | --- | --- |
| B1 | `E100/1K` and `R100` cannot run on the reference host as designed. Plan 4 Task 1 declares per-subscription routes (`/__live/async/sse/:subscription`) on the Node host, and the host is `node:http` HTTP/1.1 with no TLS, so browsers will not negotiate HTTP/2. One hundred subscribed islands means one hundred SSE connections to one origin; Chromium and Firefox cap HTTP/1.1 connections per host at six. The workload stalls in the network stack, not in the runtime, and the "eight concurrent handshakes per origin" bound in DOD 34 presumes a per-subscription connection model the browser cannot deliver. | `browser/test-host/server.mjs:1,218` (`createServer` from `node:http`); Plan 4 Task 1 route table; Plan 3 Task 6 ("connection pool keys only by approved origin/transport/auth scope"); DOD 33-34. | Plans 3 and 4 must state the subscription-to-connection model explicitly. The plan's own pool wording implies multiplexing: one document-level transport per (origin, transport, auth scope) carrying N subscriptions, with the envelope's existing `stream` plus a subscription identifier selecting the target island, and subscribe/unsubscribe as control frames (WS) or a connect-time list (SSE). Under that model `R100` is one reconnect per document, so DOD 34's handshake bound needs an honest reinterpretation (across documents/tabs) recorded as a dated contract amendment like the 08-24 one. If Shawn prefers per-subscription connections, `E100/1K` must run across multiple Playwright contexts and the host must serve HTTP/2 - a larger change. Either way the decision is Shawn's; the plan cannot leave it implicit. |
| B2 | The foundation port has no way for the upload feature to bind a ready handle to the island's model. `RuntimeFeatureIslandPort` exposes `element`, `identity`, `enqueueFreshRender`, `onDispose`, `queryDirectiveOwnership`, and `writePresentationSignal` only. The locked grammar makes `live:upload` conflict with `live:model`, so the upload directive *is* the field binding - yet nothing lets it propose the opaque handle into the next action request. DOD 9 ("finalization occurs only through a deliberate authorized action") and DOD 11 are unreachable, or the implementer smuggles the handle through a presentation signal (forbidden to write component state) or a hidden input (untrusted browser text). Same class as 003's P1/P2. | `browser/src/features/contract.ts:51-60`; `fixtures/v4/directive-grammar.json` (`upload.conflicts = ["model"]`); design "only the opaque handle becomes eligible for typed component proposal/action use". | Extend the port with one typed write, e.g. `proposeUploadHandle(field, handle \| null)`, validated in core: the field must be a declared upload field for this island per metadata, the value is an opaque bounded typed handle (never bytes, grant, filename, or arbitrary JSON), and clearing on remove/cancel is the only other write. Record it as an explicit foundation revision in Plan 1 Task 5 and add the hostile test that a feature cannot write any other model field. |
| B3 | Push `browser_event` has no dispatch port either. Plan 3 Task 8 calls `this.#events.dispatchRegistered(...)`, but event routing lives in core (`browser/src/directives/events.ts`) and the port exposes nothing for it. The async artifact would have to reach into core or own event authority itself. | `contract.ts:51-60`; Plan 3 Task 8 dispatcher; `fixtures/v4/async-envelope.json` `payload_kinds` includes `browser_event`; DOD 15, 18. | Add `dispatchRegisteredEvent(event)` to the port with schema/source/target/scope/fanout/cycle validation performed in core, so the optional artifact never becomes the authority on what an event may reach. Same foundation-revision note as B2. Treat B2 and B3 together: the port was closed one capability short on each side. |
| B4 | The reference host silently becomes a second implementation of the server. Today's Node host replays canned Rust-produced fixtures (hardcoded signatures at `scenarios.mjs:30,57`); it computes no Live semantics. Uploads and streams are dynamic - per-session handles and grants, chunk hashing, conditional revisions, cleanup races, sequence positions, replay windows, heartbeats - and cannot be canned. Plan 4 Task 1 therefore has JavaScript implement the upload state machine, grant checks, quarantine, SSE/WS sessions, and continuity. Consequences: DOD 5 ("streams actual HTTP chunks") and DOD 29 ("reference host exercises real chunked HTTP upload, authorized SSE, authorized WebSocket") are satisfied by code the Rust crate never runs; `file_provider`, `UploadService`, `SseEncoder`, `WebSocketCodec`, and `AsyncBuffer` see only in-process fakes; and conventions forbid a handwritten duplicate as a second source of truth. | `browser/test-host/scenarios.mjs:1-60`; Plan 4 Task 1 ("Stream request chunks incrementally into a test-owned quarantine directory" in `.mjs`; `ws@8.18.3`); `crates/suprnova-live-test-support/Cargo.toml` (no HTTP stack); conventions "handwritten duplicate schema is not a second source of truth". | Two honest options, Shawn's call. (i) A thin Rust reference host binary in `suprnova-live-test-support` (tokio, hyper, tokio-tungstenite as test-support dependencies only; the engine crate stays executor-neutral and free of them) that serves `/__live/*` while Node keeps serving assets and scenarios. This is what the contract's phrase "reference HTTP/SSE/WebSocket host" describes and makes DOD 5/29 Rust evidence. (ii) Keep the JS host but bind it to the fixture corpus: every transition, grant decision, and envelope it emits must be selected from `transition_cases`/`continuity_cases`, and Plan 2 gains a Rust integration test that drives `UploadService` with real `http` request types and streamed bodies so DOD 5 has Rust evidence. (ii) is cheaper and weaker; it should be named as such in the plan. |

## Important findings

| ID | Finding | Evidence | Proposed remediation |
| --- | --- | --- | --- |
| I1 | File-provider I/O model is undecided. The engine is executor-neutral (`Pin<Box<dyn Future>>` everywhere, no tokio in `[dependencies]`, `std::fs` only in `conformance.rs`), yet Plan 2 Task 4 has the provider perform exclusive create, descriptor bounding, and fsync itself. Blocking `std::fs` inside boxed futures stalls the host executor; adding `tokio::fs` makes tokio a production dependency the plan never declares. | `Cargo.toml`; `src/validation/port.rs:15`, `src/component/instance.rs:21`; Plan 2 Task 4. | Split the provider: path policy, server-generated names, chunk/whole-file hashing, and state stay in the crate; raw write/fsync/remove go through a `QuarantineStore` port the host supplies. `suprnova-live-test-support` ships the tokio-backed implementation. Plan 2 Task 4 lists the dependency decision explicitly. |
| I2 | The locked v4 `upload-protocol.json` vocabulary is not Plan 2's wire enum. Fixture operations are ledger transitions (`queue`, `begin_transfer`, `put_chunk`, `complete`, `accept`, `begin_finalize`, `commit_finalize`, `cancel`, `reject`, `expire`). Plan 2 Task 2's `UploadOperation` is wire-level (`Create`, `PutChunk`, `Status`, `Complete`, `Cancel`, `Reacquire`). Both layers are legitimate, but the plan treats them as one and lists "v4 upload fixtures" as a file it modifies - a silent change to a fixture the foundation gate just locked. | `fixtures/v4/upload-protocol.json` `operations`; Plan 2 Task 2. | Plan 2 Task 2 defines two typed layers, wire `UploadOperation` and ledger `UploadTransition`, with a mapping table (`Complete` -> `complete` then `accept`/`reject`, and so on). The fixture gains a `wire_cases` section under an explained contract change; existing `transition_cases` bytes stay untouched and a test proves it. |
| I3 | `Reacquire` is modeled as a Live protocol operation with a fixed `/__live/uploads/:handle/reacquire` route. The contract and design say cross-reload reacquisition is "an explicit authenticated application route", not ambient Live authority. Handles are permitted in HTML and snapshots by design; a Live-namespace endpoint that mints a grant from session plus handle makes handle leakage inside a session worth something. The file-identity match in Plan 2 Task 8 mitigates transfer but not cancel. | Plan 2 Task 2 enum; Plan 4 Task 1 routes; `iterations/004.md` "Cross-reload reacquisition is an explicit authenticated application route, not ambient browser authority". | Keep `UploadService::reacquire` as a server-side capability that the *application's* route calls under its own authorization. The browser reaches it only through an application-supplied port, never a fixed `/__live/` route. The reference host may demonstrate one, labeled as an example application route outside the Live namespace. If Shawn prefers the convenience of a built-in route, the contract sentence changes, not the implementation silently. |
| I4 | Cross-site WebSocket hijacking is not addressed. Plan 3 Task 6 correctly keeps bearer credentials out of URLs and lets native `EventSource` use cookie auth. WebSocket upgrades also carry cookies and are exempt from CORS; nothing in Plan 3 Tasks 4/6, Plan 4 Task 4's adversarial list, or the host requires Origin validation on the handshake. Spec 07 delegates origin to host middleware, but the upgrade is not a request the CSRF middleware sees. | Plan 3 Tasks 4, 6; Plan 4 Task 4; `07-security-and-trust-boundaries.md:66-81`. | `AuthorizedSubscription` carries the verified origin; the host adapter contract requires an Origin allowlist (or bearer-only auth) for WebSocket; the adversarial matrix and the reference host gain "cross-origin WS handshake rejected" and "cross-origin SSE-with-cookie cannot be read". |
| I5 | `live:poll` targets an `action` in the locked grammar, but Plan 3 implements polling as `enqueueFreshRender("poll")` only. Either the value is dead or polling can fire an arbitrary registered action on a timer - push-triggered mutation by another name, the exact hazard DOD 18 forbids for push. | `directive-grammar.json` (`poll.value = "action"`); Plan 3 Task 7; `14-events-and-asynchronous-updates.md` polling criteria. | Decide once: restrict poll's target to the fresh-render intent (value names the refresh), or allow only actions whose metadata declares them non-mutating/idempotent. The checker (Plan 4 Task 2) enforces the choice. |
| I6 | Hybrid mode has no source for its fallback interval. `stream` carries `push-only \| hybrid`; intervals exist only on `poll` (`5s \| 15s \| 30s \| 60s`). The contract makes hybrid the *default*, so `live:stream="orders"` alone must poll on continuity loss with an interval nobody declared. Plan 3 Task 7 and Plan 4 Task 2 ("legal poll/stream mode combinations") leave the table undefined. | `directive-grammar.json`; `iterations/004.md` "In the default hybrid policy"; Plan 3 Task 7. | Define the combination table and put it in the fixture: stream alone; stream plus poll (hybrid with poll's interval); `.push-only` plus poll (checker conflict); `.hybrid` without poll (checker error, or the descriptor's reconnect policy supplies the fallback interval). Pick one and make the checker and the runtime read the same rows. |
| I7 | DOD 1 reuse is unbound. The Rust foundation exports `ResourceOwner`, `ResourceQueue`, `BoundedQueue`, `PermitPool`, `CancellationFlag`; neither Plan 2 nor Plan 3 names them, and Plan 3 Task 5 introduces `AsyncBuffer`/`AsyncBounds` as a fresh bounded queue. Two bounded-queue implementations is the outcome the shared foundation exists to prevent. | `src/resource/*` public items; Plans 2/3 file lists (no `src/resource`); Plan 3 Task 5. | Each plan states which foundation types back service admission, chunk permits, cleanup batches, and the async buffer. Anything the foundation cannot express becomes a foundation extension with its own test, not a parallel implementation. |
| I8 | Bounded media metadata parsing has no format list, no dependency decision, and no fuzz target. Plan 2 Task 6 says "bounded image/media headers"; its fuzz list is `upload_control` and `upload_transition` only. Conventions give every external parser a fuzz target, and a crate such as `image` needs provenance review before entering the tree. | Plan 2 Tasks 6, 10; conventions "Property and fuzz tests own external parsers"; spec 08 "Image or media metadata parsing is bounded". | Name the formats (dimension-only header parsing for PNG/JPEG/GIF/WebP is the honest minimum), the byte bound per format, hand-written versus dependency, and add `fuzz/fuzz_targets/upload_media_header.rs`. |
| I9 | Plans 2 and 3 "may run independently" in the same worktree on the same branch. They share at least fourteen modify targets (`src/lib.rs`, `error.rs`, `limits.rs`, `host/capabilities.rs`, `host/context.rs`, `metadata/component.rs`, `metadata/digest.rs`, `runtime/diagnostics.ts`, `signals/lifecycle.ts`, `test-host/server.mjs`, `scenarios.mjs`, test-support `host.rs`, `fuzz/Cargo.toml`, `package.json`). Two agents on one checkout clobber each other; two worktrees produce a merge nobody planned. | Plan 2 and Plan 3 "Dependencies and execution rules". | Run 2 then 3 sequentially, or assign separate worktrees and branch names with a stated merge order and owner. Given the recent subagent incident, this needs to be explicit rather than assumed. |

## Minor findings

| ID | Finding | Proposed remediation |
| --- | --- | --- |
| M1 | Plan 4 cites `19-performance-compatibility-and-operations.md`; the file is `19-developer-tooling-and-testing.md`. Plan 4's modify list includes `docs/implementation/README.md`, which does not exist (the index lives in the root `README.md`). | Fix both references. |
| M2 | Plan 2 Task 9 "never assign `input.value`" collides with spec 08's "removal updates component state without forging native file input values". Clearing (`value = ""`) on remove or replace is not forging and is the only way to reset a file input short of replacing the node. | Word it as "never set a non-empty value and never set `files`"; clearing on remove/replace is permitted and tested. |
| M3 | Open SSE/WebSocket connections keep a page out of bfcache in Chromium unless closed before `pagehide`. Plan 3 Task 6 says "suspend closes or pauses by policy"; if the default is pause, DOD 24's bfcache tests will pass on pages that were never restored. | Make close-on-`pagehide(persisted)` the tested default and assert restoration actually occurred. |
| M4 | Gate wall time. Four new Playwright specs across three engines and two script kinds, a 64 MiB browser upload, and a ten-second `E100/1K` timeline land in the same unattended gate. | Not a defect. Budget it, and keep the `B1` runs of `U4/16` and `E100/1K` in the release path rather than per-commit. |

## Coverage challenges performed

- Read the amended contract, the approved design, all four plans, specs 08 and
  14, and `conventions.md` in full; read the iteration-003 adversarial review
  for the house standard.
- Verified every path in the four plans' create/modify lists against the
  worktree; all exist except the two in M1.
- Read the shipped `RuntimeFeatureIslandPort` and `BoundedOwner` surfaces and
  the `src/resource` public items, and compared them against what Plans 2 and 3
  call (B2, B3, I7). The browser `BoundedOwner` calls the plans make
  (`enqueue`, `acquire`, `retire`, `snapshot`) match the shipped API; the six
  post-hoc hardening commits (`1d87113` through `3833b9e`) added permit waiters,
  deferred resources, and suspension semantics beyond the plan sketch without
  changing that surface.
- Read the locked v4 fixtures (`directive-grammar.json`,
  `upload-protocol.json`, `async-envelope.json`) and compared their vocabulary
  against the plans (I2, I5, I6).
- Inspected the Node test host and the test-support crate to establish how
  server behavior is produced today (fixture replay) and what the plans would
  turn it into (B4), and confirmed the host is HTTP/1.1 (B1).
- Checked the engine's async style and dependency set for the file provider's
  I/O model (I1) and confirmed no HTTP stack exists on the Rust side.
- Confirmed `/home/shawn/workspace2/suprnova-magnetar` exists, so Plan 4's
  read-only baseline commands will not error.

## Not checked

- No test, build, or gate was run for this review; all findings come from
  reading plans, fixtures, and source. The foundation gate's green status is
  taken from the commit message `820cde2`, not re-executed.
- The full body of `features/contract.ts` (600+ lines) and `host.ts` was not
  read beyond the exported port surface; B2/B3 rest on the port's public shape.
- Specs 09, 12, 13, and 19 were consulted only where a plan cites them, not
  read end to end.

## Residual implementation risks

These remain verification obligations once the findings above are folded in:

1. The multiplexed-connection model (B1) changes what "subscription" means at
   the envelope level; the v4 `async-envelope.json` cases need a subscription
   identifier before Plan 3 Task 3 locks the codec.
2. Whichever B4 option is chosen, the Rust `file_provider` must be driven with
   a real streamed body at least once outside the Node host, or DOD 5 is a
   claim about JavaScript.
3. Grant-sentinel scans (DOD 2) must include the Node host's inspection
   counters and Playwright trace/HAR output, which are the two places a grant
   most plausibly leaks during testing.
4. `E100/1K`'s 8 KiB-per-subscription retention has no documented subtraction
   method yet for native transport buffers; the 003 review's item 5 applies
   verbatim.

## Conclusion

The foundation is in good shape and the plans are unusually complete on
bounds, redaction, and lifecycle. What they miss is structural rather than
careless: the port was sealed one capability short on each side, the workload
numbers assume a connection model the browser will not honor over HTTP/1.1,
and the reference host quietly changes character from fixture replay to a
second server. Each of B1-B4 has a bounded fix; B1 and B4 contain decisions
that belong to Shawn, and the plan should carry those decisions as dated
amendments rather than resolve them in code.

## Maintainer disposition

Recorded 2026-08-24 after the review was surfaced. Shawn explicitly ratified
all five open architecture choices and approved the complete remediation sweep.

| ID | Disposition | Locked resolution |
| --- | --- | --- |
| B1 | Accepted | Compatible logical subscriptions multiplex over one physical document transport. `E100/1K` uses exactly one connection; `R100` performs exactly one reconnect; the eight-per-origin cap is proved separately across multiple documents. |
| B2 | Accepted | Add the typed `proposeUploadHandle` feature-port write with core field/handle validation and no generic model-write seam. |
| B3 | Accepted | Add typed `dispatchRegisteredEvent` with schema/source/target/scope/fanout/cycle validation in core. |
| B4 | Accepted | A thin Rust binary in test support owns every dynamic reference route and invokes the real engine services. Node owns static production artifacts and deterministic scenario pages only. |
| I1 | Accepted | Engine-owned upload policy uses the host-supplied `QuarantineStore`; the Tokio filesystem adapter remains in test support. |
| I2 | Partially accepted; evidence corrected | The existing fixture already separates wire `operations`/`codec_cases` from internal `transition_cases`. Preserve those bytes and add an exhaustive typed wire-to-transition mapping; do not redesign the fixture around the review's proposed `wire_cases`. |
| I3 | Accepted | Reacquisition remains a server capability reached through an authenticated application-owned route outside `/__live/`. |
| I4 | Accepted | WebSocket Origin is verified before upgrade; cross-origin use requires an explicit allowlist and separate non-cookie credential. |
| I5–I6 | Accepted | `live:poll` has no action value. The signed descriptor supplies hybrid fallback policy, a legal poll directive may override it, and push-only plus poll is a conflict. |
| I7 | Accepted | Upload and async policy reuse the shared resource owner, queue, permit, and cancellation primitives; a proved missing primitive is extended at the foundation. |
| I8 | Accepted | Use exact `imagesize` 0.15.0 with default features disabled and only PNG/JPEG/GIF/WebP enabled, capped header prefixes, dependency provenance, and a dedicated fuzz target. |
| I9 | Accepted | Execute Plans 2 and 3 sequentially in the shared worktree. |
| M1–M4 | Accepted | Correct paths; permit only empty-string file-input clearing; prove real bfcache teardown/restoration; run reduced deterministic workloads normally and full qualified workloads only in release mode. |

These resolutions are normative in the amended Iteration 004 contract and
owning specs. The four implementation plans must implement them without
substituting a different connection topology, host authority boundary, upload
I/O model, polling grammar, or workload qualification rule.
