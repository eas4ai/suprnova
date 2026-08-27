/* global document, EventSource, HTMLElement, MutationObserver, window */

import {
  BrowserAsyncTransportPorts,
  configureAsync,
} from "http://127.0.0.1:4173/assets/suprnova-live.async.esm.js";
import { boot } from "http://127.0.0.1:4173/assets/suprnova-live.esm.js";

const rustOrigin = "http://127.0.0.1:4174";
const identityFacts = Object.freeze({
  principal: "task9-principal",
  scope: "task9-reference-document",
  session: "task9-session",
});
const resetResponse = await fetch(`${rustOrigin}/control/reset`, { method: "POST" });
if (!resetResponse.ok) throw new Error("async_reference_reset_failed");

function currentRoot() {
  const root = document.querySelector("[data-suprnova-live-island]");
  return root instanceof HTMLElement ? root : null;
}

if (currentRoot() === null) throw new Error("async_lifecycle_markup_missing");

const observations = {
  activeConnections: 0,
  announcements: [],
  authorizations: 0,
  authorizationCompletions: 0,
  authorizationFailures: 0,
  authorityTrace: [],
  closedConnections: 0,
  closeSignals: 0,
  connections: 0,
  continuityProofs: 0,
  cspViolations: [],
  currentSignals: 0,
  lateMessages: 0,
  liveActions: 0,
  liveRegionMutations: [],
  membershipControls: 0,
  pagehidePersisted: [],
  pageshowPersisted: [],
  resources: {
    activeAuthorizations: 0,
    buffers: 0,
    connections: 0,
    listeners: 0,
    observers: 0,
    queuedWork: 0,
    timers: 0,
  },
  resourcePeaks: {
    activeAuthorizations: 0,
    buffers: 0,
    connections: 0,
    listeners: 0,
    observers: 0,
    queuedWork: 0,
    timers: 0,
  },
  states: [],
};

function pushBounded(values, value) {
  if (values.length === 64) values.shift();
  values.push(value);
}

function recordResourcePeak(name) {
  observations.resourcePeaks[name] = Math.max(
    observations.resourcePeaks[name],
    observations.resources[name],
  );
}

function queueStart() {
  observations.resources.queuedWork += 1;
  recordResourcePeak("queuedWork");
}

function queueFinish() {
  observations.resources.queuedWork = Math.max(0, observations.resources.queuedWork - 1);
}

const stateObserver = new MutationObserver((records) => {
  for (const record of records) {
    if (!(record.target instanceof HTMLElement)) continue;
    const previous = record.oldValue;
    const current = record.target.getAttribute("data-live-stream-state");
    if (typeof previous === "string") pushBounded(observations.states, previous);
    if (typeof current === "string") pushBounded(observations.states, current);
    if (current === "current" && previous !== "current") {
      observations.currentSignals += 1;
      observations.continuityProofs += 1;
    }
  }
});
stateObserver.observe(document.documentElement, {
  attributeFilter: ["data-live-stream-state"],
  attributeOldValue: true,
  attributes: true,
  subtree: true,
});
observations.resources.observers += 1;
recordResourcePeak("observers");
pushBounded(observations.states, currentRoot()?.getAttribute("data-live-stream-state") ?? "");

const liveRegionObserver = new MutationObserver((records) => {
  for (const record of records) {
    const actionResult = document.querySelector("#async-action-result");
    if (actionResult?.textContent === "Live action committed") {
      void refreshDiagnostics();
    }
    const element =
      record.target instanceof HTMLElement
        ? record.target.closest("[data-live-stream-status]")
        : record.target.parentElement?.closest("[data-live-stream-status]");
    if (!(element instanceof HTMLElement)) continue;
    const value = element.textContent?.trim() ?? "";
    pushBounded(observations.liveRegionMutations, value);
    if (value.length !== 0) pushBounded(observations.announcements, value);
  }
});
liveRegionObserver.observe(document.documentElement, { childList: true, subtree: true });
observations.resources.observers += 1;
recordResourcePeak("observers");

function onSecurityPolicyViolation(event) {
  pushBounded(observations.cspViolations, {
    blocked: event.blockedURI,
    directive: event.effectiveDirective,
  });
}
document.addEventListener("securitypolicyviolation", onSecurityPolicyViolation);
observations.resources.listeners += 1;
recordResourcePeak("listeners");

let diagnosticsPending = false;
async function refreshDiagnostics() {
  if (diagnosticsPending) return;
  diagnosticsPending = true;
  queueStart();
  try {
    const response = await fetch(`${rustOrigin}/diagnostics`, { cache: "no-store" });
    if (response.ok) {
      const value = await response.json();
      observations.liveActions = value.live_actions;
    }
  } finally {
    queueFinish();
    diagnosticsPending = false;
  }
}

const timeoutHandles = new Set();
const timers = Object.freeze({
  clearTimeout(handle) {
    if (timeoutHandles.delete(handle)) observations.resources.timers -= 1;
    window.clearTimeout(handle);
  },
  timeout(callback, milliseconds) {
    let handle = 0;
    handle = window.setTimeout(() => {
      if (timeoutHandles.delete(handle)) observations.resources.timers -= 1;
      callback();
    }, milliseconds);
    timeoutHandles.add(handle);
    observations.resources.timers += 1;
    recordResourcePeak("timers");
    return handle;
  },
});

function connectionClosed() {
  observations.closedConnections += 1;
  observations.activeConnections = Math.max(0, observations.activeConnections - 1);
  observations.resources.connections = observations.activeConnections;
  recordResourcePeak("connections");
}

function trackedStreamResponse(response, close) {
  if (response.body === null) {
    close();
    return response;
  }
  const reader = response.body.getReader();
  let buffered = false;
  const releaseBuffer = () => {
    if (!buffered) return;
    buffered = false;
    observations.resources.buffers = Math.max(0, observations.resources.buffers - 1);
  };
  const body = new ReadableStream({
    async pull(controller) {
      releaseBuffer();
      try {
        const chunk = await reader.read();
        if (chunk.done) {
          releaseBuffer();
          close();
          controller.close();
        } else {
          buffered = true;
          observations.resources.buffers += 1;
          recordResourcePeak("buffers");
          controller.enqueue(chunk.value);
        }
      } catch (error) {
        releaseBuffer();
        close();
        controller.error(error);
      }
    },
    async cancel(reason) {
      releaseBuffer();
      close();
      await reader.cancel(reason);
    },
  });
  return new Response(body, {
    headers: response.headers,
    status: response.status,
    statusText: response.statusText,
  });
}

async function trackedFetch(input, init) {
  const url = String(input);
  if (url.endsWith("/live")) {
    const response = await fetch(input, init);
    if (response.ok && typeof init?.body === "string") {
      try {
        const request = JSON.parse(init.body);
        if (
          request.operations?.some(
            (operation) => operation.kind === "invoke_action" && operation.name === "save",
          )
        ) {
          observations.liveActions += 1;
        }
      } catch {
        // Production response validation remains the authority for malformed traffic.
      }
    }
    return response;
  }
  if (!url.endsWith("/__live/async/events")) return fetch(input, init);
  observations.connections += 1;
  observations.activeConnections += 1;
  observations.resources.connections = observations.activeConnections;
  recordResourcePeak("connections");
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    connectionClosed();
  };
  init?.signal?.addEventListener("abort", close, { once: true });
  try {
    const response = await fetch(input, init);
    return trackedStreamResponse(response, close);
  } catch (error) {
    close();
    throw error;
  }
}

const descriptorBySubscription = new WeakMap();

function authorizedSubscription(value) {
  const source = value.subscription;
  const subscription = Object.freeze({
    authorization: Object.freeze({
      credential: source.authorization.credential,
      kind: source.authorization.kind,
    }),
    baseline: Object.freeze({
      epoch: BigInt(source.baseline.epoch),
      sequence: BigInt(source.baseline.sequence),
    }),
    descriptorBinding: source.descriptor_binding,
    document: Object.freeze({
      authorizationScope: source.document.authorization_scope,
      origin: source.document.origin,
      transport: source.document.transport,
    }),
    events: Object.freeze(source.events),
    expiresAt: source.expires_at,
    fallbackPoll: Object.freeze({
      initial: source.fallback_poll.initial,
      intervalMs: source.fallback_poll.interval_ms,
      jitterRatio: source.fallback_poll.jitter_ratio,
      visibility: source.fallback_poll.visibility,
    }),
    heartbeatTimeoutMs: source.heartbeat_timeout_ms,
    presentationSignals: Object.freeze(source.presentation_signals),
    reconnect: Object.freeze({
      kind: source.reconnect.kind,
      maximumAttempts: source.reconnect.maximum_attempts,
      maximumDelayMs: source.reconnect.maximum_delay_ms,
      minimumDelayMs: source.reconnect.minimum_delay_ms,
    }),
    stream: source.stream,
    subscriptionId: source.subscription_id,
  });
  descriptorBySubscription.set(subscription, source.descriptor);
  return Object.freeze({
    replay: Object.freeze(value.replay),
    subscription,
  });
}

async function authorize(request) {
  observations.authorizations += 1;
  observations.resources.activeAuthorizations += 1;
  recordResourcePeak("activeAuthorizations");
  const body = {
    position:
      request.position === null
        ? null
        : {
            epoch: String(request.position.epoch),
            sequence: String(request.position.sequence),
          },
    prior_subscription_id: request.prior?.subscriptionId ?? null,
    ...identityFacts,
    stream: request.stream,
  };
  try {
    const response = await fetch(`${rustOrigin}/authorize`, {
      body: JSON.stringify(body),
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      method: "POST",
      signal: request.signal,
    });
    if (!response.ok) throw new Error("async_reference_authority_failed");
    const value = await response.json();
    observations.authorizationCompletions += 1;
    pushBounded(observations.authorityTrace, {
      position: body.position?.sequence ?? null,
      prior: body.prior_subscription_id,
      proof: value.proof,
      replay: value.replay.length,
    });
    return authorizedSubscription(value);
  } catch (error) {
    observations.authorizationFailures += 1;
    throw error;
  } finally {
    observations.resources.activeAuthorizations -= 1;
  }
}

const transports = new BrowserAsyncTransportPorts({
  eventSource(url, init) {
    return new EventSource(url, init);
  },
  fetch: trackedFetch,
  membershipTimeoutMs: 3_000,
  async sseMembership(request) {
    observations.membershipControls += 1;
    queueStart();
    try {
      const descriptor = descriptorBySubscription.get(request.subscription);
      if (typeof descriptor !== "string") {
        return Object.freeze({ kind: "rejected", reason: "authorization_lost" });
      }
      const response = await fetch(`${rustOrigin}/membership`, {
        body: JSON.stringify({
          control_nonce: request.controlNonce,
          descriptor,
          descriptor_binding: request.subscription.descriptorBinding,
          operation: request.operation,
          ...identityFacts,
          stream: request.subscription.stream,
          subscription_id: request.subscription.subscriptionId,
          transport_generation: request.transportGeneration,
        }),
        headers: {
          Authorization: `SuprnovaAsync ${request.subscription.authorization.credential}`,
          "Content-Type": "application/json",
        },
        method: "POST",
        signal: request.signal,
      });
      if (!response.ok) {
        return Object.freeze({ kind: "rejected", reason: "authorization_lost" });
      }
      const acknowledgment = Object.freeze({
        ...(await response.json()),
        connection: request.connection,
      });
      if (request.operation === "subscribe" && observations.membershipControls === 1) {
        queueMicrotask(() => {
          void control("current");
        });
      }
      return acknowledgment;
    } finally {
      queueFinish();
    }
  },
  timers,
  webSocket(url) {
    return new WebSocket(url);
  },
});

configureAsync({
  authority: Object.freeze({ authorize }),
  clock: Object.freeze({ now: () => Date.now() }),
  randomness: Object.freeze({ number: () => 0.5 }),
  timers,
  transports,
});
const runtime = boot();

async function control(name) {
  queueStart();
  try {
    const response = await fetch(`${rustOrigin}/control/${name}`, { method: "POST" });
    if (!response.ok) throw new Error("async_reference_control_failed");
    return response;
  } finally {
    queueFinish();
  }
}

async function injectLate() {
  const response = await control("late");
  const result = await response.json();
  observations.lateMessages += result.attempts;
}

function onDocumentClick(event) {
  const target = event.target instanceof HTMLElement ? event.target.closest("button") : null;
  if (!(target instanceof HTMLElement)) return;
  if (target.id === "degrade-stream") void control("degrade");
  if (target.id === "reconnect-stream") void control("reconnect");
  if (target.id === "close-stream") {
    void control("close").then(() => {
      observations.closeSignals += 1;
    });
  }
  if (target.id === "remove-island") {
    currentRoot()?.remove();
    timers.timeout(() => {
      void injectLate();
    }, 0);
  }
}
document.addEventListener("click", onDocumentClick);
observations.resources.listeners += 1;
recordResourcePeak("listeners");

function onPageHide(event) {
  pushBounded(observations.pagehidePersisted, event.persisted === true);
}
window.addEventListener("pagehide", onPageHide);
observations.resources.listeners += 1;
recordResourcePeak("listeners");

function onPageShow(event) {
  pushBounded(observations.pageshowPersisted, event.persisted === true);
  if (event.persisted === true) {
    timers.timeout(() => {
      void injectLate();
    }, 0);
  }
}
window.addEventListener("pageshow", onPageShow);
observations.resources.listeners += 1;
recordResourcePeak("listeners");

async function shutdown() {
  runtime.stop();
  await new Promise((resolve) => window.setTimeout(resolve, 0));
  await injectLate();
}

Reflect.set(
  window,
  "__suprnovaAsyncLifecycle",
  Object.freeze({
    shutdown,
    snapshot() {
      return structuredClone(observations);
    },
  }),
);
