# Iteration 004 Asynchronous Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver authorized typed asynchronous updates over polling, SSE, and WebSocket with bounded continuity, backpressure, reconnect, scheduler-mediated refresh, registered presentation events/signals, and no second rendering or action protocol.

**Architecture:** Component metadata declares typed events and subscription capabilities. The server signs bounded subscription descriptors that bind authorization context, topics, event schemas, baseline epoch/sequence, expiry, and reconnect policy; transport tokens remain separate secrets. SSE and WebSocket adapt one independent async envelope. The optional browser artifact maintains a bounded per-document connection pool and per-island subscription state machine. Push may enqueue an existing fresh render, dispatch a registered browser event, or set a declared presentation-only signal. Polling is complete by itself; hybrid mode pauses it only when continuity is proved.

**Tech Stack:** Rust 1.91.1, serde canonical codecs, existing HMAC/HKDF infrastructure, strict TypeScript 6.0.3, native EventSource/WebSocket/fetch/visibility/connectivity APIs behind injected ports, Vitest/fast-check, Playwright, deterministic fake streams and controlled clocks.

---

## Dependencies and execution rules

- This is Plan 3 of 4. Complete `2026-08-23-iteration-004-shared-foundation.md` first.
- This plan may proceed independently of the upload plan after the shared foundation. Plan 4 requires both.
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

- [ ] Add failing tests for event name/schema/source/target/scope/order/fanout and subscription stream/topic/event/reconnect declarations, including duplicate and unbounded contracts:

  ```rust
  #[test]
  fn async_contract_is_typed_bounded_and_digest_significant() {
      let left = metadata_with_event(EventTarget::SelfIsland, 8);
      let right = metadata_with_event(EventTarget::Document, 8);
      assert_ne!(left.contract_digest(), right.contract_digest());
      assert_eq!(metadata_with_fanout(MAX_EVENT_FANOUT + 1).unwrap_err().kind(), MetadataErrorKind::InvalidEventFanout);
  }
  ```

- [ ] Run `rtk cargo test --test async_metadata --test metadata_contract`; record failure because existing `EventMetadata` only stores name/version/payload type.
- [ ] Expand event metadata and add subscription metadata using closed enums:

  ```rust
  pub enum EventSource { Component, Stream }
  pub enum EventTarget { SelfIsland, Parent, Child, NamedIsland, Document, Browser }
  pub enum EventOrder { PerSourceSequence }
  pub enum EventCyclePolicy { ForbidRepeatedIsland, MaximumHops(NonZeroU8) }

  pub struct EventMetadata {
      name: BrowserOperationName,
      version: u16,
      payload_type: TypeId,
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

  Sort and reject duplicates, require stream events to be registered, include all fields in the canonical component digest, and preserve existing component-authored event behavior through constructors with explicit defaults.

- [ ] Run metadata, action-outcome, component-harness, and digest stability tests; format and Clippy.
- [ ] Commit: `feat(async): declare typed event subscriptions`.

## Task 2: Sign descriptors and separate transport credentials

**Files:** `src/async_updates/{subscription,authorization}.rs`, host capabilities/context, subscription/authorization/security tests

- [ ] Add failing tests for descriptor field binding, authoritative baseline, expiry, topic interpolation rejection, token redaction, renewal authorization, and principal/tenant/component/stream/topic revocation:

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

- [ ] Run subscription/authorization/security tests; record failure because descriptors do not exist.
- [ ] Implement a canonical signed descriptor and secret transport credential:

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
  }

  #[derive(Zeroize, ZeroizeOnDrop)]
  pub struct TransportCredential(Zeroizing<Vec<u8>>);
  ```

  Derive the descriptor key with HKDF purpose `suprnova-live/async-subscription/v1`. The authorization port rechecks current principal/session/tenant/component/stream/topic at connect and renewal. Topics come only from registered server metadata and trusted mount parameters; directive interpolation cannot select endpoints or topics.

- [ ] Run authorization-loss, expiry/renewal, redaction, snapshot/HTML/URL/history sentinel, and hostile-context tests.
- [ ] Commit: `feat(async): sign authorized subscription descriptors`.

## Task 3: Implement the independent bounded async envelope and sequence authority

**Files:** `src/async_updates/{envelope,sequence}.rs`, envelope tests, v4 fixtures, fuzz targets

- [ ] Add failing golden/property tests for event/refresh/signal/heartbeat/completion/error envelopes, unknown majors, duplicates, gaps, epoch changes, oversized payloads, duplicate fields, and malformed operations:

  ```rust
  proptest! {
      #[test]
      fn sequence_machine_never_applies_a_gap(positions in stream_positions()) {
          let mut machine = SequenceMachine::new(authoritative_baseline());
          for position in positions {
              if let SequenceDisposition::Apply = machine.observe(position) {
                  prop_assert_eq!(position, machine.current());
              }
          }
      }
  }
  ```

- [ ] Run `rtk cargo test --test async_envelope`; record failure because the async protocol is absent.
- [ ] Implement independent async protocol v1 and a closed payload union:

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
      stream: StreamName,
      position: StreamPosition,
      payload: AsyncPayload,
  }
  ```

  Decode with exact-key/size/depth/entry/string limits before payload allocation. `SequenceMachine` ignores duplicates, applies only the next sequence in the current epoch, degrades on gaps, and requires replay proof or authoritative refresh before adopting a new baseline.

- [ ] Add both fuzz targets, run fixtures/properties/security, and prove Live action/morph versions remain `[1, 2]`.
- [ ] Commit: `feat(async): add bounded event envelope and sequence model`.

## Task 4: Build transport-neutral SSE and WebSocket sessions

**Files:** `src/async_updates/{transport,sse,websocket}.rs`, transport conformance tests, test support

- [ ] Add a shared failing conformance suite for connect/baseline/replay, ordered delivery, duplicates, gaps, heartbeat, completion, typed errors, cancellation, auth loss, limits, slow clients, and shutdown:

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

- [ ] Run `rtk cargo test --test async_transport_conformance`; record failure because transports are absent.
- [ ] Define host-neutral source/session interfaces and two wire adapters:

  ```rust
  pub trait AsyncEventSource: Send + Sync {
      fn subscribe<'a>(&'a self, request: AuthorizedSubscription<'a>)
          -> AsyncFuture<'a, Result<Box<dyn AsyncEventSession>, AsyncError>>;
  }

  pub trait AsyncEventSession: Send {
      fn baseline(&self) -> StreamPosition;
      fn next<'a>(&'a mut self) -> AsyncFuture<'a, Result<Option<AsyncEnvelope>, AsyncError>>;
      fn close<'a>(&'a mut self) -> AsyncFuture<'a, Result<CloseDisposition, AsyncError>>;
  }
  ```

  `SseEncoder` emits bounded `id`, `event`, and canonical `data` records plus heartbeat comments. `WebSocketCodec` emits one canonical text frame per envelope and rejects binary/fragment/oversize violations according to the host adapter contract. Both consume the same verified descriptor and sequence semantics.

- [ ] Run shared conformance against both transports plus ordinary HTTP endpoint regression tests and controlled shutdown.
- [ ] Commit: `feat(async): add SSE and WebSocket transport sessions`.

## Task 5: Enforce server-side fanout and backpressure bounds

**Files:** `src/async_updates/{backpressure,telemetry}.rs`, backpressure/security tests

- [ ] Add failing tests for subscription/message/payload/heartbeat/replay/fanout/buffer limits, global outage, multi-island pressure, slow clients, and low-cardinality observability:

  ```rust
  #[test]
  fn presentation_pressure_coalesces_but_sequence_gap_degrades() {
      let mut buffer = AsyncBuffer::new(AsyncBounds { max_events: 64, max_bytes: 256 * KIB, max_fanout: 100 });
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

  pub struct AsyncBounds {
      pub max_subscriptions: NonZeroUsize,
      pub max_events: NonZeroUsize,
      pub max_bytes: NonZeroUsize,
      pub max_payload_bytes: NonZeroUsize,
      pub max_replay_events: NonZeroUsize,
      pub max_fanout: NonZeroUsize,
  }
  ```

  Coalesce only semantically replaceable presentation signals/refresh requests. Never coalesce across event names, targets, epochs, or required ordered browser events. Queue overflow degrades or closes with a typed code; it never drops a required event and claims continuity.

- [ ] Run fanout, slow-client, outage, memory-bound, and telemetry tests.
- [ ] Commit: `feat(async): bound fanout and stream backpressure`.

## Task 6: Implement browser envelope validation and subscription continuity

**Files:** `browser/src/async-updates/{types,envelope,subscription,connections,continuity}.ts`, async entry points, browser tests

- [ ] Add failing fake-transport tests for authoritative initial baseline, replay proof, duplicate/gap handling, reconnect, heartbeat loss, authorization uncertainty, page suspension, late delivery, and at most eight handshakes per origin:

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
    eventSource(connect: EventSourceRequest): EventSourcePort;
    webSocket(connect: WebSocketRequest): WebSocketPort;
  }
  ```

  The connection pool keys only by approved origin/transport/auth scope, enforces eight simultaneous handshakes per origin, applies full-jitter bounded backoff from injected randomness, and validates every envelope before queue admission. Native EventSource is used only with the scoped session-cookie authorization contract; a separately issued bearer credential uses a bounded fetch-stream SSE adapter so the secret never enters a URL. Suspend closes or pauses by policy; resume reauthorizes and reestablishes currentness before applying late data.

- [ ] Register the real async feature from ESM/classic entry points. Run feature-host, lifecycle, continuity, diagnostics, and artifact budget tests.
- [ ] Commit: `feat(browser): establish bounded subscription continuity`.

## Task 7: Make polling complete and hybrid fallback continuity-aware

**Files:** `browser/src/async-updates/poll.ts`, poll/continuity tests, directive fixtures

- [ ] Add failing controlled-clock tests for interval bounds, jitter, initial/immediate, visibility, offline, overlap, stale status, cancel/retire, failure backoff, poll-only completeness, push-only degradation, and hybrid activation after continuity loss:

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

  Poll refreshes enter `enqueueFreshRender("poll")`; overlap permits at most one queued plus one in-flight refresh per island. Hidden/offline/failed states use bounded full jitter and no synchronized catch-up burst. Push-only exposes degraded state; it never silently starts polling.

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
          return this.#events.dispatchRegistered(envelope.payload.event);
        case "presentation_signal":
          return this.#signals.setFromStream(envelope.payload.signal);
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

  Refresh uses `createFreshRenderIntent` and existing response validation/morph/commit-after-morph/fresh-render recovery. Add a semantic coalescing key so each island retains at most one queued plus one in-flight async refresh. EventRouter validates registered schema/source/target/scope/fanout/cycle before DOM dispatch; SignalRuntime accepts declared presentation-only signals only.

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
  await page.evaluate(() => window.__liveScenario.freeze());
  await page.evaluate(() => window.__liveScenario.resume());
  await expect
    .poll(() => connectionCounts(page))
    .toEqual({ streams: 1, polls: 0, timers: 1 });
  ```

- [ ] Run focused Chromium Playwright; record failure because real async scenarios are absent.
- [ ] Project state through existing semantic feedback/local signals, throttle live-region announcements, keep native controls/routes/actions usable, and bind every connection/timer/listener/buffer to the island/document resource ledger. Implement deterministic test-host endpoints for SSE, WebSocket, and poll with injected schedules; production artifacts only are served.
- [ ] Run Vitest plus Chromium/Firefox/WebKit async specs, axe checks, CSP, lifecycle, bfcache, and leak assertions.
- [ ] Commit: `test(browser): prove async lifecycle and accessibility`.

## Task 10: Fuzz, verify, and hand off asynchronous updates

**Files:** async fuzz targets and every async file

- [ ] Add envelope/sequence fuzz targets that cannot panic, allocate past limits, apply gaps, regress positions, or invent currentness:

  ```rust
  fuzz_target!(|bytes: &[u8]| {
      if let Ok(envelope) = decode_async_envelope(bytes, AsyncCodecLimits::hostile_test()) {
          let mut sequence = SequenceMachine::new(authoritative_baseline());
          if matches!(sequence.observe(envelope.position()), SequenceDisposition::Apply) {
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

- [ ] Initial connect and reconnect cannot claim current without descriptor baseline plus replay proof or authoritative refresh.
- [ ] Stream credentials are secret and separate from signed descriptors.
- [ ] Poll-only is complete; push-only reports degradation; hybrid fallback is continuity-aware and jittered.
- [ ] Push has exactly three productive effects: registered refresh, registered browser event, or declared presentation signal.
- [ ] Refresh uses the existing scheduler and protocol v2 response machine; no streamed HTML or second snapshot protocol exists.
- [ ] Buffers, fanout, handshakes, timers, connections, payloads, replay, and retained bytes have hard tested bounds.
