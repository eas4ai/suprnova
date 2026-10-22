import { describe, expect, it, vi } from "vitest";

import { createTopicSubscription, subscribeTopics } from "../src/async-updates/topics.js";
import {
  activeAsyncDocumentOwner,
  asyncFeature,
  configureAsync,
  type AsyncAuthorizationRequest,
} from "../src/async-updates/feature.js";
import type {
  DocumentMembershipOutcome,
  DocumentTransportConnectRequest,
  DocumentTransportPort,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  AsyncRuntimeIslandPort,
  RegisteredBrowserEventCapability,
} from "../src/features/contract.js";
import { eventLoopBarrier } from "./support/event-loop-barrier.js";

class SharedWebSocket implements DocumentTransportPort {
  readonly close = vi.fn();
  readonly subscribe = vi.fn(
    (
      subscription: AuthorizedLogicalSubscription,
    ): DocumentMembershipOutcome | Promise<DocumentMembershipOutcome> =>
      Object.freeze({
        descriptorBinding: subscription.descriptorBinding,
        kind: "authenticated" as const,
        stream: subscription.stream,
        subscriptionId: subscription.subscriptionId,
        transportGeneration: this.request.transportGeneration,
      }),
  );
  readonly unsubscribe = vi.fn();

  constructor(readonly request: DocumentTransportConnectRequest) {}

  open(): void {
    this.request.opened();
  }
}

function authorization(request: AsyncAuthorizationRequest): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: `binding-${request.identity.slot}`,
    document: Object.freeze({
      authorizationScope: "shared-document",
      origin: "https://app.example.test",
      transport: "websocket" as const,
    }),
    events: Object.freeze([]),
    expiresAt: 20_000,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 5_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 4_000,
      minimumDelayMs: 100,
    }),
    stream: request.stream,
    subscriptionId: `subscription-${request.identity.slot}`,
  });
}

function topicBinding(topic: string, slot: string) {
  return {
    [topic]: {
      island: {
        component: "kit.dashboard",
        document_key: "dashboard",
        slot,
      },
      stream: "changes",
    },
  };
}

function fixture() {
  const ports = new Map<string, AsyncRuntimeIslandPort>();
  const disposeController = vi.fn();
  const activateStream = vi.fn((port: AsyncRuntimeIslandPort) => {
    ports.set(port.identity.slot, port);
    return { dispose: disposeController };
  });
  const dispose = vi.fn();
  const owner = { activateStream, dispose };
  const onPoll = vi.fn();
  const onSignal = vi.fn();
  const onState = vi.fn();
  const subscription = createTopicSubscription(
    {
      notes: {
        island: {
          component: "kit.note-stats",
          document_key: "dashboard-notes",
          slot: "stats",
        },
        stream: "changes",
      },
    },
    { onPoll, onSignal, onState },
    owner,
    "document-instance-1",
  );
  const port = ports.get("stats");
  if (port === undefined) throw new Error("topic_port_missing");
  return {
    activateStream,
    dispose,
    disposeController,
    onPoll,
    onSignal,
    onState,
    port,
    subscription,
  };
}

describe("Inertia topic subscriptions", () => {
  it("attaches every topic when a page has 33 bindings", () => {
    const dispose = vi.fn();
    const activateStream = vi.fn(() => ({ dispose }));
    const bindings = Object.fromEntries(
      Array.from({ length: 33 }, (_, index) =>
        Object.entries(topicBinding(`topic-${String(index)}`, `slot-${String(index)}`)),
      ).flat(),
    );
    const subscription = createTopicSubscription(
      bindings,
      { onPoll: vi.fn(), onSignal: vi.fn() },
      { activateStream },
      "document-instance-1",
    );
    expect(activateStream).toHaveBeenCalledTimes(33);
    subscription.close();
    expect(dispose).toHaveBeenCalledTimes(33);
  });

  it("maps page bindings onto the existing async owner and forwards data-free changes", () => {
    const app = fixture();

    expect(app.activateStream).toHaveBeenCalledWith(app.port, "changes", null, []);
    expect(app.port.identity).toEqual({
      component: "kit.note-stats",
      documentKey: "dashboard-notes",
      slot: "stats",
    });
    app.port.dispatchTopicChange?.({
      cursor: 12,
      kind: "topic_change",
      props: ["stats"],
      topic: "notes",
    });
    expect(app.onSignal).toHaveBeenCalledWith({
      cursor: 12,
      props: ["stats"],
      topic: "notes",
    });

    app.port.projectAsyncStatus?.("current");
    expect(app.onState).toHaveBeenCalledWith("notes", "current");
    const complete = vi.fn();
    app.port.enqueueFreshRender("poll", complete);
    expect(app.onPoll).toHaveBeenCalledWith("notes", complete);
    const recover = vi.fn();
    expect(app.port.enqueueFreshRender("stream", recover)).toBe("queued");
    expect(app.onPoll).toHaveBeenCalledWith("notes", recover);
  });

  it("rejects a cross-topic payload and closes only its controller", () => {
    const app = fixture();

    expect(() =>
      app.port.dispatchTopicChange?.({
        cursor: 12,
        kind: "topic_change",
        props: ["stats"],
        topic: "another-topic",
      }),
    ).toThrow("topic_subscription_mismatch");
    expect(app.onSignal).not.toHaveBeenCalled();

    app.subscription.close();
    app.subscription.close();
    expect(app.disposeController).toHaveBeenCalledOnce();
    expect(app.dispose).not.toHaveBeenCalled();
    expect(app.subscription.documentInstance).toBe("document-instance-1");
  });

  it("moves an empty-event SSE fallback onto the active document WebSocket", async () => {
    const sockets: SharedWebSocket[] = [];
    let nextTimer = 0;
    const fallbackFetch = vi.fn<typeof fetch>(() => new Promise<Response>(() => undefined));
    vi.stubGlobal("window", {
      clearTimeout: vi.fn(),
      setTimeout: vi.fn(() => 1),
    });
    vi.stubGlobal("location", { origin: "https://app.example.test" });
    vi.stubGlobal("fetch", fallbackFetch);
    vi.stubGlobal("EventSource", function unavailableEventSource() {
      throw new Error("fallback_authorization_must_stay_pending");
    });
    vi.stubGlobal("WebSocket", function unavailableWebSocket() {
      throw new Error("fallback_authorization_must_stay_pending");
    });

    let connected = false;
    let island: { dispose(): void } | null = null;
    let stats: ReturnType<typeof subscribeTopics> | null = null;
    let profile: ReturnType<typeof subscribeTopics> | null = null;
    try {
      const callbacks = { onPoll: vi.fn(), onSignal: vi.fn() };
      stats = subscribeTopics(topicBinding("notes", "stats"), callbacks);
      const fallbackDocument = stats.documentInstance;
      expect(fallbackFetch).toHaveBeenCalledOnce();
      const fallbackInit = fallbackFetch.mock.calls[0]?.[1];
      if (typeof fallbackInit?.body !== "string") throw new Error("fallback_body_missing");
      expect(JSON.parse(fallbackInit.body)).toMatchObject({ transport: "sse" });

      configureAsync({
        authority: {
          authorize: (request) => authorization(request),
          documentInstance: () => "shared-websocket-document",
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: {
          clearTimeout: () => undefined,
          timeout: () => ++nextTimer,
        },
        transports: {
          eventSource() {
            throw new Error("unexpected_second_sse_transport");
          },
          webSocket(request) {
            const socket = new SharedWebSocket(request);
            sockets.push(socket);
            return socket;
          },
        },
      });
      expect(
        Reflect.apply(asyncFeature[5], asyncFeature, [
          0,
          { diagnose: vi.fn(), onDispose: vi.fn() },
        ]),
      ).toBe(true);
      connected = true;

      expect(stats.documentInstance).not.toBe(fallbackDocument);
      expect(stats.documentInstance).toBe("shared-websocket-document");
      const active = activeAsyncDocumentOwner();
      if (active === null) throw new Error("active_owner_missing");
      island = active.owner.activateStream(
        {
          consumeRegisteredEventCapability: () =>
            Object.freeze({}) as RegisteredBrowserEventCapability,
          dispatchRegisteredEvent: () => "rejected",
          element: Object.freeze({}) as Element,
          enqueueFreshRender: () => "queued",
          identity: Object.freeze({
            component: "kit.note-stats",
            documentKey: "dashboard",
            slot: "island",
          }),
          onDispose: () => undefined,
          queryDirectiveOwnership: () => Object.freeze([]),
          writePresentationSignal: (_scope, _name, value) => value,
        },
        "changes",
        null,
        [],
      );
      if (island === null) throw new Error("island_subscription_missing");
      profile = subscribeTopics(topicBinding("profile", "profile"), callbacks);
      await eventLoopBarrier();

      expect(sockets).toHaveLength(1);
      expect(profile.documentInstance).toBe("shared-websocket-document");
      sockets[0]?.open();
      await eventLoopBarrier();

      stats.close();
      stats = null;
      expect(sockets[0]?.close).not.toHaveBeenCalled();
      expect(sockets[0]?.unsubscribe).toHaveBeenCalledOnce();

      profile.close();
      profile = null;
      expect(sockets[0]?.close).not.toHaveBeenCalled();
      expect(sockets[0]?.unsubscribe).toHaveBeenCalledTimes(2);

      island.dispose();
      island = null;
      expect(sockets[0]?.close).toHaveBeenCalledOnce();
    } finally {
      stats?.close();
      profile?.close();
      island?.dispose();
      if (connected) Reflect.apply(asyncFeature[5], asyncFeature, [5, null]);
      vi.unstubAllGlobals();
    }
  });
});
