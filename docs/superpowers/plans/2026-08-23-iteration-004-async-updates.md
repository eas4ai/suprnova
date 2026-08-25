# Iteration 004 Asynchronous Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver authorized typed asynchronous updates over polling, SSE, and WebSocket with bounded continuity, backpressure, reconnect, scheduler-mediated refresh, registered presentation events/signals, and no second rendering or action protocol.

**Architecture:** Component metadata declares typed events and subscription capabilities. The server signs bounded subscription descriptors that bind authorization context, topics, event schemas, baseline epoch/sequence, expiry, reconnect policy, and the default hybrid fallback interval; transport tokens remain separate secrets. SSE and WebSocket adapt one independent async envelope. One bounded physical document transport multiplexes compatible logical subscriptions, while each island keeps an independent continuity state machine. Push may enqueue an existing fresh render, dispatch a registered browser event through the typed feature port, or set a declared presentation-only signal. Polling is complete by itself; hybrid mode pauses it only when continuity is proved.

**Tech Stack:** Rust 1.91.1, serde canonical codecs, existing HMAC/HKDF infrastructure, strict TypeScript 6.0.3, native EventSource/WebSocket/fetch/visibility/connectivity APIs behind injected ports, Vitest/fast-check, Playwright, deterministic fake streams and controlled clocks.

---

## Dependencies and execution rules

- This is Plan 3 of 4. Complete `2026-08-23-iteration-004-shared-foundation.md` first.
- Complete Plan 2, `2026-08-23-iteration-004-uploads.md`, before this plan in the shared worktree. Plans 2 and 3 are logically separable but must not execute concurrently against one checkout. Plan 4 requires both.
- Work only in `/home/shawn/workspace2/suprnova-live/.worktrees/iteration-004-uploads-async`; never push without explicit authorization.
- Start every shell command with `rtk`; use `apply_patch` for hand edits; do not use blanket `-D warnings`.
- All time, randomness, transport, visibility, online state, and lifecycle behavior must be injectable. Correctness tests use no elapsed sleeps.
- Do not add streamed HTML, arbitrary effect/action dispatch, component state writes, SPA navigation, RenderCache, or vendor/framework integration.

## File structure

### Create

- `src/async_updates/mod.rs`
- `src/async_updates/metadata.rs`
- `src/async_updates/envelope.rs`
- `src/async_updates/subscription.rs`
- `src/async_updates/authorization.rs`
- `src/async_updates/sequence.rs`
- `src/async_updates/transport.rs`
- `src/async_updates/sse.rs`
- `src/async_updates/websocket.rs`
- `src/async_updates/backpressure.rs`
- `src/async_updates/telemetry.rs`
- `src/resource/cancel.rs` (modify only if Plan 2 proves an async requirement the shared foundation does not satisfy)
- `src/resource/owner.rs` (modify only under the same rule)
- `src/resource/queue.rs` (modify only under the same rule)
- `tests/async_metadata.rs`
- `tests/async_envelope.rs`
- `tests/async_subscription.rs`
- `tests/async_transport_conformance.rs`
- `tests/async_authorization.rs`
- `tests/async_backpressure.rs`
- `tests/async_security.rs`
- `browser/src/async-updates/feature.ts`
- `browser/src/async-updates/types.ts`
- `browser/src/async-updates/envelope.ts`
- `browser/src/async-updates/subscription.ts`
- `browser/src/async-updates/connections.ts`
- `browser/src/async-updates/poll.ts`
- `browser/src/async-updates/continuity.ts`
- `browser/src/async-updates/dispatch.ts`
- `browser/src/async-updates/backpressure.ts`
- `browser/tests/async-envelope.test.ts`
- `browser/tests/async-subscription.test.ts`
- `browser/tests/async-poll.test.ts`
- `browser/tests/async-continuity.test.ts`
- `browser/tests/async-dispatch.test.ts`
- `browser/tests/async-backpressure.test.ts`
- `browser/e2e/async-updates.spec.ts`
- `fuzz/fuzz_targets/async_envelope.rs`
- `fuzz/fuzz_targets/async_sequence.rs`

### Modify

- `src/lib.rs`
- `src/metadata/browser.rs`
- `src/metadata/component.rs`
- `src/metadata/digest.rs`
- `src/host/capabilities.rs`
- `src/host/context.rs`
- `src/resource/mod.rs` (only for a proved shared-foundation extension)
- `src/error.rs`
- `browser/src/entry-async-esm.ts`
- `browser/src/entry-async-classic.ts`
- `browser/src/runtime/ports.ts`
- `browser/src/runtime/diagnostics.ts`
- `browser/src/scheduler/intent.ts`
- `browser/src/islands/record.ts`
- `browser/src/directives/events.ts`
- `browser/src/signals/lifecycle.ts`
- `browser/test-host/server.mjs`
- `browser/test-host/scenarios.mjs`
- `crates/suprnova-live-test-support/src/lib.rs`
- `crates/suprnova-live-test-support/src/host.rs`
- `fuzz/Cargo.toml`

## Task 1: Make typed event and subscription metadata digest-significant

**Files:** metadata modules, `src/async_updates/{mod,metadata}.rs`, metadata tests

- [x] Add failing tests for event name/schema/source/target/scope/order/fanout and subscription stream/topic/event/reconnect declarations, including duplicate and unbounded contracts:

  ```rust
  #[test]
  fn async_contract_is_typed_bounded_and_digest_significant() {
      let left = metadata_with_event(EventTarget::SelfIsland, 8);
      let right = metadata_with_event(EventTarget::Document, 8);
      assert_ne!(left.contract_digest(), right.contract_digest());
      assert_eq!(metadata_with_fanout(MAX_EVENT_FANOUT + 1).unwrap_err().kind(), MetadataErrorKind::InvalidEventFanout);
  }
  ```

- [x] Run `rtk cargo test --test async_metadata --test metadata_contract`; record failure because existing `EventMetadata` only stores name/version/payload type.
- [x] Expand event metadata and add subscription metadata using closed enums:

  ```rust
  pub enum EventSource { Component, Stream }
  pub enum EventTarget {
      SelfIsland,
      Parent,
      Child,
      NamedIsland(IslandSlot),
      Document,
      Browser(BrowserOperationName),
  }
    pub enum EventOrder { PerSourceSequence }
    pub enum EventCyclePolicy { ForbidRepeatedIsland, MaximumHops(NonZeroU8) }

    pub trait EventPayloadMetadata {
        const NAME: &'static str;
        const VERSION: u16;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = Self::NAME;
    }

    pub struct EventMetadata {
        name: BrowserOperationName,
        version: u16,
        payload_type: TypeId,
        payload_contract: PayloadContractIdentity,
        schema: BrowserPayloadSchema,
      source: EventSource,
      targets: BoundedTargets,
      order: EventOrder,
      cycle: EventCyclePolicy,
      maximum_fanout: NonZeroU16,
  }

  pub struct SubscriptionMetadata {
      stream: StreamName,
      topics: BoundedTopics,
      events: BoundedEventNames,
      modes: SubscriptionModes,
      reconnect: ReconnectPolicy,
  }
  ```

  Sort and reject duplicates, require stream events to be registered, include all fields in the canonical component digest, and preserve existing component-authored event behavior through constructors with explicit defaults. `TypeId` remains an in-process matching guard only; the digest uses the explicit validated `PAYLOAD_CONTRACT`, whose compatibility default is the registered event identity and whose override lets payload contracts evolve independently.

  `EventTarget` is also the closed propagation-scope contract. The named-island
  and approved-browser-listener variants carry the exact registered identity,
  so metadata cannot authorize an arbitrary named target or global listener.

- [x] Run metadata, action-outcome, component-harness, and digest stability tests; format and Clippy.
- [x] Commit: `feat(async): declare typed event subscriptions`.

## Task 2: Sign descriptors and separate transport credentials

**Files:** `src/async_updates/{subscription,authorization}.rs`, host capabilities/context, subscription/authorization/security tests

- [x] Add failing tests for descriptor field binding, authoritative baseline, expiry, topic interpolation rejection, token redaction, renewal authorization, and principal/tenant/component/stream/topic revocation:

  ```rust
  #[test]
  fn descriptor_baseline_is_authority_but_transport_token_is_separate() {
      let issued = fixture_issuer().issue(request()).unwrap();
      let verified = fixture_verifier().verify(issued.descriptor(), scope()).unwrap();
      assert_eq!(verified.baseline(), StreamPosition::new(epoch(4), sequence(19)));
      assert!(!issued.descriptor().as_str().contains(TOKEN_SENTINEL));
      assert!(!format!("{:?}", issued).contains(TOKEN_SENTINEL));
  }
  ```

- [x] Run subscription/authorization/security tests; record failure because descriptors do not exist.
- [x] Implement a canonical signed descriptor and secret transport credential:

  ```rust
  pub struct SubscriptionClaims {
      stream: StreamName,
      protocol: u16,
      capability: CapabilityVersion,
      topics: BoundedTopics,
      events: BoundedEventContracts,
      authorization_memo: AuthorizationMemo,
      baseline: StreamPosition,
      expires_at: UnixMillis,
      reconnect: ReconnectPolicy,
      fallback_poll: PollFallbackPolicy,
  }

  #[derive(Zeroize, ZeroizeOnDrop)]
  pub struct TransportCredential(Zeroizing<Vec<u8>>);
  ```

  `PollFallbackPolicy` carries a bounded interval, jitter, initial behavior, and
  visibility policy. It is the authoritative default for hybrid subscriptions;
  a legal empty-valued `live:poll` may override it, while `push-only` plus
  `live:poll` is a directive conflict. Derive the descriptor key with HKDF
  purpose `suprnova-live/async-subscription/v1`. The authorization port rechecks
  current principal/session/tenant/component/stream/topic at connect and
  renewal. Topics come only from registered server metadata and trusted mount
  parameters; directive interpolation cannot select endpoints or topics.
  The public issue request cannot propose a baseline: current resolved scope is
  sent to a trusted continuity port. Trusted registration calculates the
  worst-case full canonical claims and rejects metadata that cannot fit the
  descriptor budget. After every non-mutating check, one distributed/restart-safe
  credential-provider operation consumes the predecessor and persists a unique
  unpredictable successor: Connect becomes Renew and Renew becomes Connect.
  Provider failure retains the predecessor; a committed-but-lost response
  requires a freshly issued subscription and does not introduce Task 3
  idempotent outcome machinery.

- [x] Run authorization-loss, expiry/renewal, redaction, snapshot/HTML/URL/history sentinel, and hostile-context tests.
- [x] Commit: `feat(async): sign authorized subscription descriptors`.

## Task 3: Implement the independent bounded async envelope and sequence authority

**Files:** `src/async_updates/{envelope,sequence}.rs`, envelope tests, v4 fixtures, fuzz targets

- [x] Add failing golden/property tests for event/refresh/signal/heartbeat/completion/error envelopes, unknown majors, duplicates, gaps, epoch changes, oversized payloads, duplicate fields, and malformed operations:

  ```rust
  proptest! {
      #[test]
      fn sequence_machine_never_applies_a_gap(positions in stream_positions()) {
          let mut machine = SequenceMachine::new(&sealed_authorized_context());
          for position in positions {
              let guard = freshly_admit(position);
              if let Ok(SequenceDisposition::Apply) = machine.dispatch(guard, now, &mut dispatcher()) {
                  prop_assert_eq!(position, machine.current());
              }
          }
      }
  }
  ```

- [x] Run `rtk cargo test --test async_envelope`; record failure because the async protocol is absent.
- [x] Implement independent async protocol v1 and a closed payload union:

  ```rust
  pub const SUPPORTED_ASYNC_PROTOCOL_VERSIONS: &[u16] = &[1];

  pub enum AsyncPayload {
      Refresh(RegisteredRefresh),
      BrowserEvent(RegisteredBrowserEvent),
      PresentationSignal(RegisteredPresentationSignal),
      Heartbeat(Heartbeat),
      Complete(CompletionReason),
      Error(StreamErrorCode),
  }

  pub struct AsyncEnvelope {
      protocol: u16,
      subscription: SubscriptionId,
      stream: StreamName,
      position: StreamPosition,
      payload: AsyncPayload,
  }
  ```

  `subscription` is required because compatible logical subscriptions share a
  physical document transport; it is validated against that transport's active
  membership before sequence observation or dispatch. Decode with
  exact-key/size/depth/entry/string limits before payload allocation.
  `SequenceMachine` ignores duplicates, applies only the next sequence in the
  current epoch, degrades on gaps, and requires a bounded contiguous transcript
  of already validated same-scope envelopes through recorded high-water or an
  authoritative host refresh before adopting a new baseline. Epoch adoption is
  available only through the injected host continuity authority. Its initial
  position comes only from the verified signed baseline retained by the sealed
  Task 2 authorization context; construction has no raw position argument.
  That cloneable context is static codec authority. Each sequence observation
  consumes a fresh one-use host membership guard, and exact-next position commits
  only after closed registered dispatch succeeds. Replay prevalidates the whole
  bounded transcript and reports/retains a truthful committed prefix on failure.

- [x] Add both fuzz targets, run fixtures/properties/security, and prove Live action/morph versions remain `[1, 2]`.
- [x] Commit: `feat(async): add bounded event envelope and sequence model`.

## Task 4: Build transport-neutral logical sessions and multiplexed document transports

**Files:** `src/async_updates/{transport,sse,websocket}.rs`, transport conformance tests, test support

- [x] Add a shared failing conformance suite for connect/baseline/replay, ordered delivery, subscription routing, duplicates, gaps, heartbeat, completion, typed errors, cancellation, auth loss, strict WebSocket origin validation, logical membership, limits, slow clients, and shutdown:

  ```rust
  pub async fn assert_async_transport(factory: impl AsyncTransportFactory) {
      let mut session = factory.connect(authorized_descriptor()).await.unwrap();
      assert_eq!(session.baseline(), authoritative_baseline());
      assert_ordered_delivery(&mut session).await;
      assert_gap_degrades_without_apply(&mut session).await;
      assert_slow_client_is_bounded(&mut session).await;
      session.close().await.unwrap();
      assert_eq!(session.close().await.unwrap(), CloseDisposition::AlreadyClosed);
  }
  ```

- [x] Run `rtk cargo test --test async_transport_conformance`; record failure because transports are absent.
- [x] Define host-neutral source/session interfaces and two wire adapters:

  ```rust
  pub trait AsyncEventSource: Send + Sync {
      fn subscribe<'a>(&'a self, request: AuthorizedSubscription<'a>)
          -> AsyncFuture<'a, Result<Box<dyn AsyncEventSession>, AsyncError>>;
  }

  pub trait AsyncEventSession: Send {
      fn baseline(&self) -> StreamPosition;
      fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>)
          -> Poll<Result<Option<AsyncEnvelope>, AsyncError>>;
      fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>)
          -> Poll<Result<CloseDisposition, AsyncError>>;
  }

  pub struct DocumentTransportSession {
      origin: VerifiedOrigin,
      transport: DocumentTransportKind,
      memberships: BoundedSubscriptionMemberships,
      retiring: BoundedLogicalSessions,
  }
  ```

  `AsyncEventSource` and `AsyncEventSession` remain the host-neutral interfaces
  for one authorized logical subscription. `DocumentTransportSession` is the
  bounded fan-in layer: it routes each envelope by exact currently authorized
  `SubscriptionId` and never merges logical sequence authority. The ID itself
  remains non-secret routing identity and grants no authority.

  `AuthorizedTransportSubscription` is a descriptor-bound request, not a
  reusable admission result. Every external add/remove calls the trusted
  `AsyncTransportAuthorityPort` with the exact subscription, operation, origin,
  document kind, and correlation-only handle. The framework compares one fresh
  current host snapshot for component/identity authorization memo, stream,
  topics, full event contracts, and canonical registered modes and independently
  checks exclusive descriptor expiry. Control is split so no mutable or shared
  document borrow crosses an await: synchronous preparation snapshots the exact
  document tuple, exact server-owned instance, and generation; owned fresh authority runs; a synchronous
  pre-source gate checks generation, duplicate/retirement fences, and capacity;
  owned source establishment plus post-authority runs; and a one-use synchronous
  commit repeats the exact checks before mutation. Failed/canceled
  post-validation or stale commit closes/disposes an opened session once and
  installs nothing. Pending controls own hard-bounded RAII permits. Internal
  retirement and shutdown remain independent from expired browser authority.

  The connect-authorized result retains a compact redacted binding of the exact
  signed descriptor wire. Duplicate, remove, and control paths compare that
  binding, so equal claims signed under overlapping key IDs remain distinct.
  Physical sharing instead uses a trusted `DocumentAuthorizationScope` derived
  from aggregate scope, session, principal, tenant, and host transport policy;
  component identity remains in each logical authorization memo. Active and
  retiring logical sessions share one hard bound; a retiring entry retains its
  exact ID and binding fence until cleanup succeeds. Completion detaches before
  its one terminal envelope, typed Error payloads remain nonterminal, and
  persistent fair close polling prevents pending/failing cleanup from stalling
  siblings.

  `SseEncoder` emits bounded `id`, `event`, and canonical `data` records plus
  heartbeat comments. SSE membership changes use authenticated same-origin
  control requests and a non-authoritative document transport handle.
  `WebSocketCodec` emits canonical text frames for envelopes and bounded
  subscribe/unsubscribe control records, and rejects binary, fragmentation,
  noncanonical syntax, and oversize violations. Decoding never consults local
  membership state; fresh authority precedes unknown/binding classification at
  synchronous unsubscribe commit. Before upgrade, the host must
  validate `Origin` against the application origin; cross-origin WebSockets
  require an explicit allowlist and a separate non-cookie credential. A missing,
  malformed, wildcard-authorized, or unapproved origin is rejected before any
  subscription credential is accepted. Both adapters consume the same verified
  descriptor and sequence semantics.

  WebSocket raw frame shape and byte ceilings are checked before UTF-8 and JSON
  work (65,536 envelope bytes; 512 control bytes). Registered
  `SubscriptionModes` are authority rather than a document-kind hint, so exact
  mode-set drift revokes a retained request.

- [x] Run shared conformance against both transports, cross-site WebSocket
  hijacking cases, membership add/remove/replay cases, ordinary HTTP endpoint
  regression tests, and controlled shutdown. The complete common semantic suite
  runs through genuine SSE response/HTTP-control/record paths and genuine
  WebSocket origin/control/envelope paths; adapter-tagged counters prove both
  executed every case. Controlled clocks and barriers cover preflight and
  post-subscribe expiry, revocation, scope/mode drift, once-only disposal, and
  unauthorized external removal without sleeps. They also prove document
  delivery and independent controls progress during pending authority/source
  work, stale one-use commits clean up, active-plus-retiring fences survive key
  overlap, WebSocket denial reveals no membership oracle, hard pending-control
  bounds release on drop, and controlled pending reads/closes register and wake
  exact waiters.
- [x] Commit: `feat(async): add SSE and WebSocket transport sessions`.

## Task 5: Enforce server-side fanout and backpressure bounds

**Files:** `src/async_updates/{backpressure,telemetry}.rs`, backpressure/security tests

- [ ] Add failing tests for subscription/message/payload/heartbeat/replay/fanout/buffer limits, global outage, multi-island pressure, slow clients, and low-cardinality observability:

  ```rust
  #[test]
  fn presentation_pressure_coalesces_but_sequence_gap_degrades() {
      let owner = ResourceOwner::new(ResourceBounds::new(64, 256 * KIB).unwrap());
      let permits = PermitPool::new(8).unwrap();
      let mut buffer = AsyncBackpressure::new(owner, permits, AsyncPolicy { max_fanout: 100 });
      for event in repeated_signal_events(1_000) { buffer.offer(event).unwrap(); }
      assert!(buffer.retained_events() <= 64);
      assert!(buffer.retained_bytes() <= 256 * KIB);
      assert_eq!(buffer.offer(gapped_refresh()).unwrap(), BufferDisposition::Degraded);
  }
  ```

- [ ] Run backpressure/security tests; record failure because buffer/fanout policies are absent.
- [ ] Implement typed dispositions:

  ```rust
  pub enum BufferDisposition {
      Queued,
      Coalesced,
      Degraded,
      Closed(AsyncCloseCode),
  }

  pub struct AsyncPolicy {
      pub max_payload_bytes: NonZeroUsize,
      pub max_replay_events: NonZeroUsize,
      pub max_fanout: NonZeroUsize,
  }
  ```

  `AsyncBackpressure` is a policy wrapper around the shared
  `ResourceOwner`/`BoundedQueue`, `PermitPool`, and `CancellationFlag`; it must
  not implement a second private queue, permit counter, cancellation primitive,
  or lifecycle owner. Extend the shared foundation only when a failing
  cross-feature test proves a missing primitive, then rerun both upload and async
  resource tests. Coalesce only semantically replaceable presentation
  signals/refresh requests. Never coalesce across subscription IDs, event names,
  targets, epochs, or required ordered browser events. Queue overflow degrades
  or closes with a typed code; it never drops a required event and claims
  continuity.

- [ ] Run fanout, slow-client, outage, memory-bound, and telemetry tests.
- [ ] Commit: `feat(async): bound fanout and stream backpressure`.

## Task 6: Implement browser envelope validation and subscription continuity

**Files:** `browser/src/async-updates/{types,envelope,subscription,connections,continuity}.ts`, async entry points, browser tests

- [ ] Add failing fake-transport tests for authoritative initial baseline, complete validated replay transcripts, subscription routing, duplicate/gap handling, reconnect, heartbeat loss, authorization uncertainty, page suspension, late delivery, 100 logical subscriptions sharing one document transport, and at most eight concurrent handshakes per origin across multiple documents:

  ```ts
  it("cannot claim current on initial connect without proof", () => {
    const subscription = fixtureSubscription({ baseline: position(3, 40) });
    subscription.connected();
    expect(subscription.state()).toBe("connecting");
    subscription.receive(envelope(position(3, 41), refreshPayload()));
    expect(subscription.state()).toBe("current");
    subscription.receive(envelope(position(3, 43), signalPayload()));
    expect(subscription.state()).toBe("degraded");
    expect(appliedSignals()).toHaveLength(0);
  });
  ```

- [ ] Run async envelope/subscription/continuity tests; record failure because the async feature is inert.
- [ ] Implement the state machine and injected transport ports:

  ```ts
  export type SubscriptionState =
    | "disconnected"
    | "connecting"
    | "current"
    | "degraded"
    | "reconnecting"
    | "closed";

  export interface AsyncTransportPorts {
    eventSource(connect: DocumentEventSourceRequest): EventSourcePort;
    webSocket(connect: DocumentWebSocketRequest): WebSocketPort;
  }

  export interface DocumentTransportPort {
    subscribe(subscription: AuthorizedLogicalSubscription): void;
    unsubscribe(subscriptionId: string): void;
    close(reason: DocumentTransportCloseReason): void;
  }
  ```

  `DocumentTransportKey` contains only approved origin, transport, and auth
  scope. One physical port per key multiplexes its bounded logical membership;
  islands retain independent descriptor, sequence, and continuity state. The
  scheduler enforces at most eight concurrent handshakes per origin across
  document transports and applies full-jitter bounded backoff from injected
  randomness. E100/1K must show that 100 logical subscriptions use exactly one
  physical connection.

  Validate the envelope and its active subscription membership before queue
  admission. Native EventSource is used only with the scoped session-cookie
  authorization contract; a separately issued bearer credential uses a bounded
  fetch-stream SSE adapter so the secret never enters a URL. A persisted
  `pagehide` closes physical transports and timers rather than merely pausing
  them. `pageshow` after bfcache restoration obtains current authorization,
  establishes a new physical transport, and proves continuity before accepting
  late data.

- [ ] Register the real async feature from ESM/classic entry points. Run feature-host, lifecycle, continuity, diagnostics, and artifact budget tests.
- [ ] Commit: `feat(browser): establish bounded subscription continuity`.

## Task 7: Make polling complete and hybrid fallback continuity-aware

**Files:** `browser/src/async-updates/poll.ts`, poll/continuity tests, directive fixtures

- [ ] Add failing controlled-clock tests for interval bounds, jitter, initial/immediate, visibility, offline, overlap, stale status, cancel/retire, failure backoff, empty directive value enforcement, poll-only completeness, push-only degradation, descriptor-default hybrid fallback, legal poll override, directive conflict, and hybrid activation after continuity loss:

  ```ts
  it("hybrid pauses polling only while continuity is proved", () => {
    const fixture = hybridFixture({ intervalMs: 30_000, jitter: 0.2 });
    fixture.stream.proveCurrent(position(1, 9));
    fixture.clock.advance(60_000);
    expect(fixture.refreshes()).toBe(0);
    fixture.stream.gap(position(1, 11));
    fixture.clock.advanceToNextTimer();
    expect(fixture.refreshes()).toBe(1);
  });
  ```

- [ ] Run poll/continuity tests; record failure because polling policies are absent.
- [ ] Implement explicit policies:

  ```ts
  export interface PollPolicy {
    readonly intervalMs: number;
    readonly jitterRatio: number;
    readonly initial: "wait" | "immediate";
    readonly visibility: "visible" | "always";
    readonly mode: "poll_only" | "push_only" | "hybrid";
  }
  ```

  Consume the v4 generated freshness-combination table created in Plan 1; do
  not duplicate its rules in handwritten parser branches. `live:poll` has an
  empty value and can only configure a fresh-render timer:

  - poll without a stream is `poll_only` and uses the poll interval;
  - hybrid stream without poll uses the signed descriptor interval;
  - hybrid stream plus poll uses the poll interval as an override;
  - push-only without poll never falls back;
  - push-only plus poll is `directive_conflict`;
  - a stream with no explicit mode modifier is hybrid.

  Poll refreshes enter `enqueueFreshRender("poll")`; they never carry an action,
  effect, or arbitrary operation name. Overlap permits at most one queued plus
  one in-flight refresh per island. Hidden/offline/failed states use bounded
  full jitter and no synchronized catch-up burst. Push-only exposes degraded
  state; it never silently starts polling.

- [ ] Run poll, scheduler, connectivity, visibility, bfcache, and 100-subscription storm tests.
- [ ] Commit: `feat(browser): add complete polling and hybrid fallback`.

## Task 8: Dispatch only registered refresh, browser events, and presentation signals

**Files:** `browser/src/async-updates/dispatch.ts`, scheduler intent/island record, event/signal routers, dispatch/security tests

- [ ] Add failing hostile tests proving push cannot invoke mutating action/effect/call names, install HTML/snapshots, alter revisions/component state, exceed target fanout, or reach retired/wrong-scope islands:

  ```ts
  it("rejects every authority-writing payload", () => {
    for (const payload of [
      actionPayload(),
      effectPayload(),
      htmlPayload(),
      snapshotPayload(),
    ]) {
      expect(() => dispatcher.dispatch(payload as never)).toThrowError(
        "unsupported_async_payload",
      );
    }
    expect(serverActions()).toEqual([]);
    expect(morphs()).toEqual([]);
    expect(commits()).toEqual([]);
  });
  ```

- [ ] Run async dispatch/security tests; record failure because no dispatcher exists.
- [ ] Implement a closed dispatcher:

  ```ts
  export class AsyncDispatcher {
    dispatch(envelope: ValidatedAsyncEnvelope): DispatchDisposition {
      switch (envelope.payload.kind) {
        case "refresh":
          return this.#island.enqueueFreshRender("stream");
        case "browser_event":
          return this.#island.dispatchRegisteredEvent(envelope.payload.event);
        case "presentation_signal":
          return this.#island.writePresentationSignal(
            envelope.payload.signal.element,
            envelope.payload.signal.name,
            envelope.payload.signal.value,
          );
        case "heartbeat":
          return "observed";
        case "complete":
          return this.#subscription.close(envelope.payload.reason);
        case "error":
          return this.#subscription.degrade(envelope.payload.code);
      }
    }
  }
  ```

  The optional artifact receives only `RuntimeFeatureIslandPort`; it cannot reach
  private event or signal routers. Core validates every
  `RegisteredBrowserEventCandidate` against registered schema, source, target,
  scope, fanout, and cycle before DOM dispatch. Core likewise accepts only
  declared presentation-signal writes through the existing typed method. Do not
  add a generic dispatch, invoke, action, effect, or state-write seam.

  Refresh uses `createFreshRenderIntent` and existing response
  validation/morph/commit-after-morph/fresh-render recovery. Add a semantic
  coalescing key so each island retains at most one queued plus one in-flight
  async refresh.

- [ ] Run dispatch, scheduler, response-ordering, morph failure, event ownership, signal, and security suites.
- [ ] Commit: `feat(async): dispatch bounded presentation updates`.

## Task 9: Add exact lifecycle, accessibility, and real-browser behavior

**Files:** async feature modules, feedback/signals, test host scenarios, Playwright spec

- [ ] Add failing browser tests for disconnected/connecting/current/degraded/reconnecting/closed semantic status, bounded announcements, focus/reduced motion, pagehide/freeze/resume/pageshow/bfcache, navigation, morph replacement, island removal, shutdown, listener/connection/timer duplication, and late responses:

  ```ts
  await page.goto("/async/hybrid");
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await page.goto("/plain");
  await page.goBack();
  await expect.poll(() => page.evaluate(() => window.__liveScenario.lastPageShowPersisted)).toBe(true);
  await expect
    .poll(() => connectionCounts(page))
    .toEqual({ streams: 1, polls: 0, timers: 1 });
  ```

- [ ] Run focused Chromium Playwright; record failure because real async scenarios are absent.
- [ ] Project state through existing semantic feedback/local signals, throttle live-region announcements, keep native controls/routes/actions usable, and bind every connection/timer/listener/buffer to the island/document resource ledger. On every persisted `pagehide`, close physical transports, cancel timers, and retire listeners/buffers exactly once. On persisted `pageshow`, reauthorize, create a new transport, and prove continuity before applying data. The Playwright scenario must observe `PageTransitionEvent.persisted === true`; a synthetic freeze/resume call is supplementary evidence, not bfcache proof.
- [ ] Add deterministic static scenario descriptions and fault schedules for Plan 4's thin Rust reference host. Node serves only static scenario pages/assets and never implements subscription authority, SSE/WebSocket state, continuity, or polling semantics. Production artifacts only are served.
- [ ] Run Vitest plus Chromium/Firefox/WebKit async specs, axe checks, CSP, lifecycle, bfcache, and leak assertions.
- [ ] Commit: `test(browser): prove async lifecycle and accessibility`.

## Task 10: Fuzz, verify, and hand off asynchronous updates

**Files:** async fuzz targets and every async file

- [ ] Add envelope/sequence fuzz targets that cannot panic, allocate past limits, apply gaps, regress positions, or invent currentness:

  ```rust
  fuzz_target!(|bytes: &[u8]| {
      if let Ok(envelope) = decode_async_envelope(bytes, AsyncCodecLimits::hostile_test()) {
          let context = sealed_authorized_context();
          let mut sequence = SequenceMachine::new(&context);
          let guard = context.freshly_admit(&envelope);
          if matches!(sequence.dispatch(guard, now, &mut dispatcher()), Ok(SequenceDisposition::Apply)) {
              assert_eq!(envelope.position(), sequence.current());
          }
      }
  });
  ```

- [ ] Run the complete async gate:

  ```bash
  rtk cargo fmt --all -- --check
  rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features
  rtk env CARGO_INCREMENTAL=0 cargo test --test async_metadata --test async_envelope --test async_subscription --test async_transport_conformance --test async_authorization --test async_backpressure --test async_security
  rtk cargo +nightly fuzz build
  rtk npm --prefix browser run format:check
  rtk npm --prefix browser run lint
  rtk npm --prefix browser run typecheck
  rtk npm --prefix browser test -- async-envelope.test.ts async-subscription.test.ts async-poll.test.ts async-continuity.test.ts async-dispatch.test.ts async-backpressure.test.ts
  rtk npm --prefix browser run test:browser -- --project=chromium async-updates.spec.ts
  rtk npm --prefix browser run build:check
  rtk npm --prefix browser run budget
  rtk git diff --check
  ```

- [ ] Inspect every descriptor/token/diagnostic/log/trace path, confirm no endpoint/topic interpolation from directives, verify ordinary HTTP Live still works with async missing or incompatible, and commit verification corrections as `chore: close iteration 004 async gate`.

## Definition-of-done coverage

- DOD 15–18: Tasks 1–5 and 8 cover typed metadata, signed authorization, transport-neutral semantics, and the closed push authority surface.
- DOD 19–24: Tasks 6–9 cover scheduler refresh, polling/hybrid behavior, continuity, bounds, lifecycle, and exact cleanup.
- DOD 25, 27, 29–31: Tasks 8–10 cover semantic feedback, independent versioning, real transport/reference-host evidence, fuzz/security/adversarial cases.
- DOD 33–34: Tasks 5–10 establish the E100/1K and R100 behavior measured and hard-gated in Plan 4.

## Plan self-review checklist

- [ ] Initial connect and reconnect cannot claim current without descriptor baseline plus a complete validated replay transcript or authoritative host refresh.
- [ ] Stream credentials are secret and separate from signed descriptors.
- [ ] Poll-only is complete; push-only reports degradation; hybrid fallback is continuity-aware and jittered.
- [ ] `live:poll` carries no action value; the signed descriptor supplies hybrid fallback and a legal poll directive only overrides its interval policy.
- [ ] Push has exactly three productive effects: registered refresh, registered browser event, or declared presentation signal.
- [ ] Browser-event dispatch crosses the typed core feature port and is validated there; no optional artifact receives a generic authority-writing seam.
- [ ] Refresh uses the existing scheduler and protocol v2 response machine; no streamed HTML or second snapshot protocol exists.
- [ ] WebSocket upgrade rejects missing or unapproved origins before credentials; approved cross-origin use requires an explicit allowlist and separate non-cookie credential.
- [ ] Async policy wraps the shared bounded-resource foundation rather than implementing a second queue, owner, permit pool, or cancellation model.
- [ ] One document transport multiplexes compatible logical subscriptions; E100/1K uses one physical connection, R100 performs one reconnect handshake, and a separate multi-document test proves the eight-per-origin handshake bound.
- [ ] Persisted pagehide always closes transports and timers; persisted pageshow reauthorizes and reestablishes continuity without duplicate resources.
- [ ] Buffers, fanout, handshakes, timers, connections, payloads, replay, and retained bytes have hard tested bounds.
