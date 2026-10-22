import type { JsonValue } from "../canonical.js";
import type {
  AsyncRuntimeIslandPort,
  FeatureIslandController,
  RegisteredBrowserEventCapability,
  RuntimeFeatureDocumentContext,
} from "../features/contract.js";
import type {
  FreshRenderCompletionObserver,
  FreshRenderDisposition,
  FreshRenderReason,
} from "../features/host.js";
import { BrowserAsyncAuthority, browserAsyncOptions } from "./browser-host.js";
import {
  activeAsyncDocumentOwner,
  AsyncDocumentOwner,
  observeAsyncDocumentOwner,
  type ActiveAsyncDocumentOwner,
} from "./feature.js";
import type { AsyncPayload, SubscriptionState } from "./types.js";

const CONTRACT_NAME = /^[A-Za-z0-9._:/-]+$/u;

export interface TopicSubscriptionIsland {
  readonly component: string;
  readonly document_key: string;
  readonly slot: string;
}

export interface TopicSubscriptionBinding {
  readonly island: TopicSubscriptionIsland;
  readonly stream: string;
}

export type TopicSubscriptionBindings = Readonly<Record<string, TopicSubscriptionBinding>>;

export interface TopicChangeSignal {
  readonly cursor: number;
  readonly props: readonly string[];
  readonly topic: string;
}

export interface TopicSubscriptionCallbacks {
  onPoll(topic: string, completion?: FreshRenderCompletionObserver): void;
  onSignal(signal: TopicChangeSignal): void;
  onState?(topic: string, state: SubscriptionState): void;
}

export interface TopicSubscription {
  close(): void;
  readonly documentInstance: string;
}

export interface TopicSubscriptionOwner {
  activateStream(
    port: AsyncRuntimeIslandPort,
    stream: string,
    pollModifiers: readonly string[] | null,
    streamModifiers: readonly string[],
  ): FeatureIslandController | null;
}

function contractName(value: unknown, maximum: number): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= maximum &&
    CONTRACT_NAME.test(value)
  );
}

function bindingEntries(bindings: unknown): [string, TopicSubscriptionBinding][] {
  if (bindings === null || typeof bindings !== "object" || Array.isArray(bindings)) {
    throw new Error("topic_bindings_invalid");
  }
  const entries = Object.entries(bindings);
  if (entries.length === 0) throw new Error("topic_bindings_invalid");
  const validated: [string, TopicSubscriptionBinding][] = [];
  for (const [topic, value] of entries) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("topic_bindings_invalid");
    }
    const island: unknown = Reflect.get(value, "island");
    if (
      !contractName(topic, 256) ||
      topic
        .split("/")
        .some((segment) => segment.length === 0 || segment === "." || segment === "..") ||
      !contractName(Reflect.get(value, "stream"), 128) ||
      island === null ||
      typeof island !== "object" ||
      Array.isArray(island) ||
      !contractName(Reflect.get(island, "component"), 256) ||
      !contractName(Reflect.get(island, "document_key"), 256) ||
      !contractName(Reflect.get(island, "slot"), 256)
    ) {
      throw new Error("topic_bindings_invalid");
    }
    validated.push([topic, value as TopicSubscriptionBinding]);
  }
  return validated;
}

function validCallbacks(value: unknown): value is TopicSubscriptionCallbacks {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const callbacks = value as Partial<TopicSubscriptionCallbacks>;
  return (
    typeof callbacks.onPoll === "function" &&
    typeof callbacks.onSignal === "function" &&
    (callbacks.onState === undefined || typeof callbacks.onState === "function")
  );
}

function unusedCapability(): RegisteredBrowserEventCapability {
  return Object.freeze({}) as RegisteredBrowserEventCapability;
}

function topicPort(
  topic: string,
  binding: TopicSubscriptionBinding,
  callbacks: TopicSubscriptionCallbacks,
): AsyncRuntimeIslandPort {
  return Object.freeze({
    consumeRegisteredEventCapability: unusedCapability,
    dispatchRegisteredEvent: () => "rejected" as const,
    dispatchTopicChange(change: Extract<AsyncPayload, { kind: "topic_change" }>) {
      if (change.topic !== topic) throw new Error("topic_subscription_mismatch");
      callbacks.onSignal(
        Object.freeze({ cursor: change.cursor, props: change.props, topic: change.topic }),
      );
    },
    element: Object.freeze({}) as Element,
    enqueueFreshRender(
      _reason: FreshRenderReason,
      completion?: FreshRenderCompletionObserver,
    ): FreshRenderDisposition {
      callbacks.onPoll(topic, completion);
      return "queued";
    },
    identity: Object.freeze({
      component: binding.island.component,
      documentKey: binding.island.document_key,
      slot: binding.island.slot,
    }),
    onDispose: () => undefined,
    projectAsyncStatus(state: SubscriptionState) {
      callbacks.onState?.(topic, state);
    },
    queryDirectiveOwnership: () => Object.freeze([]),
    writePresentationSignal(_scope: string, _name: string, value: JsonValue): JsonValue {
      return value;
    },
  });
}

export function createTopicSubscription(
  bindings: TopicSubscriptionBindings,
  callbacks: TopicSubscriptionCallbacks,
  owner: TopicSubscriptionOwner,
  documentInstance: string,
): TopicSubscription {
  if (!validCallbacks(callbacks) || !contractName(documentInstance, 128)) {
    throw new Error("topic_subscription_invalid");
  }
  const controllers: FeatureIslandController[] = [];
  try {
    for (const [topic, binding] of bindingEntries(bindings)) {
      const controller = owner.activateStream(
        topicPort(topic, binding, callbacks),
        binding.stream,
        null,
        [],
      );
      if (controller === null) throw new Error("topic_subscription_unavailable");
      controllers.push(controller);
    }
  } catch (error) {
    for (const controller of controllers.reverse()) controller.dispose();
    throw error;
  }
  let closed = false;
  return Object.freeze({
    close() {
      if (closed) return;
      closed = true;
      for (const controller of controllers.reverse()) controller.dispose();
    },
    documentInstance,
  });
}

function documentContext(): RuntimeFeatureDocumentContext {
  return Object.freeze({
    diagnose: () => undefined,
    onDispose: () => undefined,
  });
}

interface TopicOwnerLease extends ActiveAsyncDocumentOwner {
  release(): void;
}

interface StandaloneTopicOwner extends ActiveAsyncDocumentOwner {
  references: number;
}

let standaloneOwner: StandaloneTopicOwner | null = null;

function activeOwnerLease(active: ActiveAsyncDocumentOwner): TopicOwnerLease {
  return Object.freeze({
    ...active,
    release: () => undefined,
  });
}

function standaloneOwnerLease(): TopicOwnerLease {
  if (standaloneOwner === null) {
    const options = browserAsyncOptions({ transport: "sse" });
    const authority = options.authority;
    if (!(authority instanceof BrowserAsyncAuthority)) {
      throw new Error("topic_subscription_unavailable");
    }
    standaloneOwner = {
      documentInstance: authority.documentInstance(),
      owner: new AsyncDocumentOwner(documentContext(), options),
      references: 0,
    };
  }
  const acquired = standaloneOwner;
  acquired.references += 1;
  let released = false;
  return Object.freeze({
    documentInstance: acquired.documentInstance,
    owner: acquired.owner,
    release() {
      if (released) return;
      released = true;
      acquired.references -= 1;
      if (acquired.references !== 0 || standaloneOwner !== acquired) return;
      standaloneOwner = null;
      acquired.owner.dispose();
    },
  });
}

function topicOwnerLease(): TopicOwnerLease {
  const active = activeAsyncDocumentOwner();
  return active === null ? standaloneOwnerLease() : activeOwnerLease(active);
}

/** Subscribe an Inertia page through Live's existing authority and transport pool. */
export function subscribeTopics(
  bindings: TopicSubscriptionBindings,
  callbacks: TopicSubscriptionCallbacks,
): TopicSubscription {
  let lease = topicOwnerLease();
  let subscription: TopicSubscription;
  try {
    subscription = createTopicSubscription(
      bindings,
      callbacks,
      lease.owner,
      lease.documentInstance,
    );
  } catch (error) {
    lease.release();
    throw error;
  }

  let closed = false;
  const stopObserving = observeAsyncDocumentOwner((active) => {
    if (closed || active === null || active.owner === lease.owner) return;
    const nextLease = activeOwnerLease(active);
    let nextSubscription: TopicSubscription;
    try {
      nextSubscription = createTopicSubscription(
        bindings,
        callbacks,
        nextLease.owner,
        nextLease.documentInstance,
      );
    } catch {
      nextLease.release();
      return;
    }
    const previousSubscription = subscription;
    const previousLease = lease;
    subscription = nextSubscription;
    lease = nextLease;
    previousSubscription.close();
    previousLease.release();
  });

  return Object.freeze({
    close() {
      if (closed) return;
      closed = true;
      stopObserving();
      subscription.close();
      lease.release();
    },
    get documentInstance() {
      return subscription.documentInstance;
    },
  });
}
