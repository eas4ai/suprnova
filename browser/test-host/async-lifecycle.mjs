/* global document, HTMLElement, MutationObserver, window */

import { BrowserAsyncTransportPorts, configureAsync } from "/assets/suprnova-live.async.esm.js";
import { boot } from "/assets/suprnova-live.esm.js";

const rustOrigin = "http://127.0.0.1:4174";
const resetResponse = await fetch(`${rustOrigin}/control/reset`, { method: "POST" });
if (!resetResponse.ok) throw new Error("async_reference_reset_failed");
const root = document.querySelector("[data-suprnova-live-island]");
const status = document.querySelector("[data-live-stream-status]");
if (!(root instanceof HTMLElement) || !(status instanceof HTMLElement)) {
  throw new Error("async_lifecycle_markup_missing");
}

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
  currentSignals: 0,
  lateMessages: 0,
  pagehidePersisted: [],
  pageshowPersisted: [],
  states: [],
};

function pushUnique(values, value) {
  if (typeof value === "string" && value.length !== 0 && !values.includes(value)) {
    values.push(value);
  }
}

new MutationObserver((records) => {
  for (const record of records) {
    pushUnique(observations.states, record.oldValue);
    const current = root.getAttribute("data-live-stream-state");
    pushUnique(observations.states, current);
    if (current === "current" && record.oldValue !== "current") {
      observations.continuityProofs += 1;
    }
  }
}).observe(root, {
  attributeFilter: ["data-live-stream-state"],
  attributeOldValue: true,
  attributes: true,
});

new MutationObserver(() => {
  pushUnique(observations.announcements, status.textContent?.trim() ?? "");
}).observe(status, { childList: true, subtree: true });

window.addEventListener("pagehide", (event) => {
  observations.pagehidePersisted.push(event.persisted === true);
});
window.addEventListener("pageshow", (event) => {
  observations.pageshowPersisted.push(event.persisted === true);
});

const timers = Object.freeze({
  clearTimeout(handle) {
    window.clearTimeout(handle);
  },
  timeout(callback, milliseconds) {
    return window.setTimeout(callback, milliseconds);
  },
});

async function trackedFetch(input, init) {
  const url = String(input);
  if (!url.endsWith("/__live/async/events")) return fetch(input, init);
  observations.connections += 1;
  observations.activeConnections += 1;
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    observations.closedConnections += 1;
    observations.activeConnections -= 1;
  };
  init?.signal?.addEventListener("abort", close, { once: true });
  try {
    return await fetch(input, init);
  } catch (error) {
    close();
    throw error;
  }
}

async function authorize(request) {
  observations.authorizations += 1;
  const sequence = request.position?.sequence ?? 0n;
  const url = new URL("/authorize", rustOrigin);
  url.searchParams.set("sequence", String(sequence));
  url.searchParams.set("prior", request.prior?.subscriptionId ?? "");
  let value;
  try {
    const response = await fetch(url, { cache: "no-store", signal: request.signal });
    if (!response.ok) throw new Error("async_reference_authority_failed");
    value = await response.json();
    observations.authorizationCompletions += 1;
    if (observations.authorityTrace.length < 32) {
      observations.authorityTrace.push({
        baseline: value.baseline.sequence,
        position: String(sequence),
        prior: request.prior?.subscriptionId ?? null,
        replay: value.replay.length,
      });
    }
  } catch (error) {
    observations.authorizationFailures += 1;
    throw error;
  }
  const subscription = Object.freeze({
    authorization: Object.freeze({
      credential: "reference-bearer-credential-0001",
      kind: "bearer",
    }),
    baseline: Object.freeze({
      epoch: BigInt(value.baseline.epoch),
      sequence: BigInt(value.baseline.sequence),
    }),
    descriptorBinding: value.descriptor_binding,
    document: Object.freeze({
      authorizationScope: "task9-reference-document",
      origin: rustOrigin,
      transport: "sse",
    }),
    events: Object.freeze([]),
    expiresAt: Date.now() + 60_000,
    fallbackPoll: Object.freeze({
      initial: "wait",
      intervalMs: 30_000,
      jitterRatio: 0,
      visibility: "visible",
    }),
    heartbeatTimeoutMs: 10_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh",
      maximumAttempts: 4,
      maximumDelayMs: 1_000,
      minimumDelayMs: 250,
    }),
    stream: value.stream,
    subscriptionId: value.subscription_id,
  });
  return Object.freeze({ replay: Object.freeze(value.replay), subscription });
}

const transports = new BrowserAsyncTransportPorts({
  eventSource() {
    throw new Error("async_reference_unexpected_cookie_transport");
  },
  fetch: trackedFetch,
  membershipTimeoutMs: 3_000,
  async sseMembership(request) {
    const membershipUrl = new URL("/membership", rustOrigin);
    membershipUrl.searchParams.set("control_nonce", request.controlNonce);
    membershipUrl.searchParams.set("binding", request.subscription.descriptorBinding);
    membershipUrl.searchParams.set("operation", request.operation);
    membershipUrl.searchParams.set("stream", request.subscription.stream);
    membershipUrl.searchParams.set("subscription", request.subscription.subscriptionId);
    membershipUrl.searchParams.set("generation", String(request.transportGeneration));
    const response = await fetch(membershipUrl, {
      method: "POST",
      signal: request.signal,
    });
    if (!response.ok) return Object.freeze({ kind: "rejected", reason: "authorization_lost" });
    const acknowledgment = Object.freeze({
      ...(await response.json()),
      connection: request.connection,
    });
    if (request.operation === "subscribe") {
      queueMicrotask(() => {
        void fetch(`${rustOrigin}/control/current`, { method: "POST" }).then((result) => {
          if (result.ok) observations.currentSignals += 1;
        });
      });
    }
    return acknowledgment;
  },
  timers,
  webSocket() {
    throw new Error("async_reference_unexpected_websocket");
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
  const response = await fetch(`${rustOrigin}/control/${name}`, { method: "POST" });
  if (!response.ok) throw new Error("async_reference_control_failed");
}

document.querySelector("#degrade-stream")?.addEventListener("click", () => {
  void control("degrade");
});
document.querySelector("#reconnect-stream")?.addEventListener("click", () => {
  void control("reconnect");
});
document.querySelector("#close-stream")?.addEventListener("click", () => {
  void control("close").then(() => {
    observations.closeSignals += 1;
  });
});
document.querySelector("#replace-island")?.addEventListener("click", () => {
  const content = root.querySelector("#async-content");
  if (content !== null) content.textContent = "Morphed async content";
});
document.querySelector("#remove-island")?.addEventListener("click", () => {
  root.remove();
});

Reflect.set(
  window,
  "__suprnovaAsyncLifecycle",
  Object.freeze({
    shutdown() {
      runtime.stop();
    },
    snapshot() {
      return structuredClone(observations);
    },
  }),
);
