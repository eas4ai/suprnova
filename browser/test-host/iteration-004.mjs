/* global document, window */

const REFERENCE_AUTHORIZATION = "Bearer task1-reference-session";
const INTERNAL_RESOURCE_COUNTS = Symbol.for("suprnova.live.internal.resource-counts.v1");
const FEATURE_SYMBOL = Symbol.for("suprnova.live.features.v1");
const features = document.documentElement.getAttribute("data-iteration-004-features") ?? "core";
const format = document.documentElement.getAttribute("data-iteration-004-format") ?? "esm";
const uploadArtifact =
  document.documentElement.getAttribute("data-iteration-004-upload-artifact") ?? "current";
const asyncArtifact =
  document.documentElement.getAttribute("data-iteration-004-async-artifact") ?? "current";
const transportKind =
  document.documentElement.getAttribute("data-iteration-004-transport") ?? "sse";
const syntheticLifecycle =
  document.documentElement.getAttribute("data-iteration-004-synthetic-lifecycle") === "true";
const controlledClock =
  document.documentElement.getAttribute("data-iteration-004-controlled-clock") === "true";
const controlledUploadClock =
  document.documentElement.getAttribute("data-iteration-004-controlled-upload-clock") === "true";
const uploadChunkBytes = Number(
  document.documentElement.getAttribute("data-iteration-004-upload-chunk-bytes") ?? "262144",
);
if (uploadChunkBytes !== 262_144 && uploadChunkBytes !== 262_145) {
  throw new Error("iteration_004_upload_chunk_bytes_invalid");
}
const hasUploads = features === "uploads" || features === "both";
const hasAsync = features === "async" || features === "both";
const originalFetch = window.fetch.bind(window);
const issuedByKind = new Map();
const requestedGenerationByKind = new Map([
  ["sse", 1],
  ["websocket", 1],
]);
const activePorts = new Set();
const observations = {
  authorizations: [],
  cspViolations: [],
  errors: [],
  featureRegistrations: [],
  freshnessStates: [],
  forwardedEnvelopes: 0,
  heldEnvelopes: 0,
  pagehidePersisted: [],
  pageshowPersisted: [],
  portsCreated: 0,
  retiredEnvelopeAttempts: 0,
  subscriptionAttempts: 0,
  transportFailures: [],
};
const heldEnvelopes = [];
let holdNextEnvelope = false;
let persistedRestart = false;
let runtime = null;
let readyUpload = null;
let pauseNextUploadChunk = false;
let pauseEveryUploadChunk = false;
let uploadPauseGeneration = null;
const controlledTimerRecords = new Map();
let controlledTimerHandle = 1_000_000;
let uploadClockNow = 0;

if (controlledUploadClock) {
  Object.defineProperty(performance, "now", {
    configurable: true,
    value: () => uploadClockNow,
  });
}

function runtimeResourceCounts() {
  if (runtime === null) return {};
  const inspect = Reflect.get(runtime, INTERNAL_RESOURCE_COUNTS);
  if (typeof inspect !== "function") throw new Error("runtime_resource_probe_missing");
  return Reflect.apply(inspect, runtime, []);
}

async function hostInspection() {
  const response = await originalFetch("/__test/iteration-004/inspection", {
    cache: "no-store",
  });
  if (!response.ok) throw new Error("reference_inspection_failed");
  return response.json();
}

function boundedPush(values, value) {
  if (values.length === 64) values.shift();
  values.push(value);
}

document.addEventListener("securitypolicyviolation", (event) => {
  boundedPush(observations.cspViolations, event.effectiveDirective);
});
window.addEventListener("error", (event) => {
  boundedPush(
    observations.errors,
    event.error instanceof Error ? event.error.message : event.message,
  );
});
window.addEventListener("unhandledrejection", (event) => {
  const reason = event.reason;
  boundedPush(observations.errors, reason instanceof Error ? reason.message : String(reason));
});
window.addEventListener("pagehide", (event) => {
  boundedPush(observations.pagehidePersisted, event.persisted === true);
  pauseNextUploadChunk = false;
  pauseEveryUploadChunk = false;
  uploadPauseGeneration = null;
  if (event.persisted === true) persistedRestart = true;
});
window.addEventListener("pageshow", (event) => {
  boundedPush(observations.pageshowPersisted, event.persisted === true);
  if (event.persisted === true) {
    const kind = transportKind === "websocket" ? "websocket" : "sse";
    requestedGenerationByKind.set(kind, 1);
  }
});

function grantFrom(headers) {
  const value = headers.get("Authorization") ?? "";
  const prefix = "SuprnovaUpload ";
  if (!value.startsWith(prefix) || value.length === prefix.length) {
    throw new Error("reference_upload_grant_missing");
  }
  return value.slice(prefix.length);
}

function mappedUploadResponse(operation, value) {
  const state = value.state === "created" ? "queued" : value.state;
  const response = { revision: String(value.revision), state };
  if (operation === "create") {
    response.grant = value.grant;
    response.handle = value.handle;
  }
  if (operation === "status") response.nextChunkIndex = value.next_part;
  return response;
}

async function pauseUploadChunk(handle, uploadRevision) {
  if (uploadPauseGeneration !== null) {
    throw new Error("iteration_004_upload_pause_in_use");
  }
  const pause = await originalFetch("/__test/iteration-004/control/upload/pause-chunk", {
    body: JSON.stringify({ handle, upload_revision: uploadRevision }),
    headers: { "Content-Type": "application/json" },
    method: "POST",
  });
  if (!pause.ok) {
    const detail = await pause.text();
    boundedPush(observations.errors, `reference_upload_pause_failed:${detail}`);
    throw new Error("reference_upload_pause_failed");
  }
  const control = await pause.json();
  uploadPauseGeneration = control.pause_generation;
}

async function referenceUploadFetch(input, init = {}) {
  const source = input instanceof Request ? input.url : String(input);
  const url = new URL(source, document.baseURI);
  if (url.origin !== window.location.origin || url.pathname !== "/__live/upload") {
    return originalFetch(input, init);
  }
  const headers = new Headers(init.headers);
  let operation = headers.get("X-Suprnova-Upload-Operation");
  let body = null;
  if (operation === null) {
    if (typeof init.body !== "string") throw new Error("reference_upload_control_invalid");
    body = JSON.parse(init.body);
    operation = body.operation;
  }
  let response;
  if (operation === "create") {
    response = await originalFetch("/__live/uploads", {
      body: JSON.stringify({
        content_type: body.file.type || "application/octet-stream",
        expected_bytes: body.file.size,
        field: body.field,
        filename: body.file.name,
        mode: "file",
      }),
      headers: {
        Authorization: REFERENCE_AUTHORIZATION,
        "Content-Type": "application/json",
      },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "put_chunk") {
    const handle = headers.get("X-Suprnova-Upload-Handle");
    const part = headers.get("X-Suprnova-Upload-Chunk");
    response = await originalFetch(`/__live/uploads/${handle}/chunks/${part}`, {
      body: init.body,
      headers: {
        Authorization: REFERENCE_AUTHORIZATION,
        "Content-Type": "application/octet-stream",
        "X-Live-Chunk-Bytes": String(init.body.byteLength),
        "X-Live-Chunk-Sha256": headers.get("X-Suprnova-Upload-Checksum") ?? "",
        "X-Live-Upload-Grant": grantFrom(headers),
      },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "complete") {
    response = await originalFetch(`/__live/uploads/${body.handle}/complete`, {
      body: JSON.stringify({ grant: grantFrom(headers) }),
      headers: {
        Authorization: REFERENCE_AUTHORIZATION,
        "Content-Type": "application/json",
      },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "cancel") {
    response = await originalFetch(`/__live/uploads/${body.handle}/cancel`, {
      headers: {
        Authorization: REFERENCE_AUTHORIZATION,
        "X-Live-Upload-Grant": grantFrom(headers),
      },
      keepalive: init.keepalive === true,
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "status") {
    response = await originalFetch(`/__live/uploads/${body.handle}`, {
      headers: {
        Authorization: REFERENCE_AUTHORIZATION,
        "X-Live-Upload-Grant": grantFrom(headers),
      },
      method: "GET",
      signal: init.signal,
    });
  } else {
    throw new Error("reference_upload_operation_invalid");
  }
  if (!response.ok) return response;
  const value = await response.json();
  if (operation === "create") {
    readyUpload = {
      expectedBytes: value.expected_bytes,
      handle: value.handle,
      readyRevision: null,
    };
    if (pauseNextUploadChunk || pauseEveryUploadChunk) {
      pauseNextUploadChunk = false;
      await pauseUploadChunk(value.handle, value.revision);
    }
  } else if (
    operation === "put_chunk" &&
    pauseEveryUploadChunk &&
    value.received_bytes < value.expected_bytes
  ) {
    await pauseUploadChunk(value.handle, value.revision);
  } else if (operation === "complete" && readyUpload?.handle === body.handle) {
    pauseEveryUploadChunk = false;
    readyUpload.readyRevision = value.revision;
  } else if (operation === "cancel" && readyUpload?.handle === body.handle) {
    pauseEveryUploadChunk = false;
    uploadPauseGeneration = null;
    readyUpload = null;
  }
  return new Response(JSON.stringify(mappedUploadResponse(operation, value)), {
    headers: { "Content-Type": "application/json" },
    status: response.status,
  });
}

function membershipRecord(issued, subscriptionId) {
  const membership = issued.memberships.find(
    (candidate) => candidate.subscription === subscriptionId,
  );
  if (membership === undefined) throw new Error("reference_membership_missing");
  return membership;
}

async function issueAuthorization(request) {
  const kind = transportKind === "websocket" ? "websocket" : "sse";
  const transportGeneration = requestedGenerationByKind.get(kind);
  if (!Number.isSafeInteger(transportGeneration) || transportGeneration < 1) {
    throw new Error("reference_transport_generation_invalid");
  }
  const response = await originalFetch("/__live/async/transports", {
    body: JSON.stringify({
      kind,
      position:
        request.position === null
          ? null
          : {
              epoch: String(request.position.epoch),
              sequence: String(request.position.sequence),
            },
      prior_subscription: request.prior?.subscriptionId ?? null,
      subscription: request.stream,
      transport_generation: transportGeneration,
    }),
    headers: {
      Authorization: REFERENCE_AUTHORIZATION,
      "Content-Type": "application/json",
    },
    method: "POST",
    signal: request.signal,
  });
  if (!response.ok) throw new Error("reference_async_authorization_failed");
  const issued = await response.json();
  issuedByKind.set(kind, issued);
  const secondary = request.identity.documentKey.includes("secondary");
  const membership = issued.memberships[secondary ? 1 : 0];
  const source = membership.browser_authorization;
  boundedPush(observations.authorizations, {
    baseline: {
      epoch: String(source.baseline.epoch),
      sequence: String(source.baseline.sequence),
    },
    position:
      request.position === null
        ? null
        : {
            epoch: String(request.position.epoch),
            sequence: String(request.position.sequence),
          },
    subscription: membership.subscription,
    transport: issued.transport,
    requestedGeneration: transportGeneration,
    serverGeneration: issued.transport_generation,
    replay: source.replay.length,
  });
  return Object.freeze({
    replay: Object.freeze(source.replay),
    subscription: Object.freeze({
      authorization: Object.freeze({
        credential: source.authorization.credential,
        kind: source.authorization.kind,
      }),
      baseline: Object.freeze({
        epoch: BigInt(source.baseline.epoch),
        sequence: BigInt(source.baseline.sequence),
      }),
      descriptorBinding: membership.descriptor_binding,
      document: Object.freeze({
        authorizationScope: source.document.authorization_scope,
        origin: window.location.origin,
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
    }),
  });
}

function deliverEnvelope(port, request, encoded) {
  if (holdNextEnvelope) {
    holdNextEnvelope = false;
    observations.heldEnvelopes += 1;
    heldEnvelopes.push({ port, request, encoded });
    return;
  }
  if (port.closed) return;
  observations.forwardedEnvelopes += 1;
  request.message(encoded);
}

class ReferenceSsePort {
  constructor(request) {
    this.closed = false;
    this.abort = new AbortController();
    this.request = request;
    this.issued = issuedByKind.get("sse");
    this.subscriptions = new Set();
    this.nextControl = 0;
    this.controlTail = Promise.resolve();
    this.readerStarted = false;
    if (this.issued === undefined) throw new Error("reference_sse_transport_missing");
    if (this.issued.transport_generation !== request.transportGeneration) {
      throw new Error("reference_sse_transport_generation_mismatch");
    }
    persistedRestart = false;
    observations.portsCreated += 1;
    activePorts.add(this);
    queueMicrotask(() => {
      if (!this.closed) request.opened();
    });
  }

  subscribe(subscription) {
    observations.subscriptionAttempts += 1;
    const operation = this.controlTail.then(() => this.attach(subscription));
    this.controlTail = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }

  async attach(subscription) {
    if (this.closed) return Object.freeze({ kind: "rejected", reason: "closed" });
    const membership = membershipRecord(this.issued, subscription.subscriptionId);
    this.nextControl += 1;
    const controlNonce = `task3-${this.issued.transport}-${String(this.nextControl).padStart(8, "0")}`;
    const response = await originalFetch(
      `/__live/async/transports/${this.issued.transport}/subscriptions/${subscription.subscriptionId}`,
      {
        body: JSON.stringify({
          authority: membership.authority,
          control_nonce: controlNonce,
          operation: "subscribe",
          transport_generation: this.request.transportGeneration,
        }),
        headers: {
          Authorization: REFERENCE_AUTHORIZATION,
          "Content-Type": "application/json",
        },
        method: "POST",
      },
    );
    if (!response.ok) {
      boundedPush(observations.transportFailures, `membership:${response.status}`);
      return Object.freeze({ kind: "rejected", reason: "authorization_lost" });
    }
    this.subscriptions.add(subscription.subscriptionId);
    if (!this.readerStarted) {
      this.readerStarted = true;
      void this.controlTail.then(() => {
        if (!this.closed) void this.read();
      });
    }
    const acknowledged = await response.json();
    return Object.freeze(acknowledged);
  }

  unsubscribe(subscriptionId) {
    if (this.closed || !this.subscriptions.delete(subscriptionId)) return;
    const membership = membershipRecord(this.issued, subscriptionId);
    this.nextControl += 1;
    const controlNonce = `task3-u-${this.issued.transport}-${String(this.nextControl).padStart(8, "0")}`;
    void originalFetch(
      `/__live/async/transports/${this.issued.transport}/subscriptions/${subscriptionId}`,
      {
        body: JSON.stringify({
          authority: membership.authority,
          control_nonce: controlNonce,
          operation: "unsubscribe",
          transport_generation: this.request.transportGeneration,
        }),
        headers: {
          Authorization: REFERENCE_AUTHORIZATION,
          "Content-Type": "application/json",
        },
        method: "POST",
      },
    );
  }

  close() {
    if (this.closed) return;
    for (const subscription of [...this.subscriptions]) this.unsubscribe(subscription);
    this.closed = true;
    this.abort.abort();
    activePorts.delete(this);
    requestedGenerationByKind.set(
      "sse",
      persistedRestart ? 1 : this.request.transportGeneration + 1,
    );
  }

  async read() {
    try {
      const response = await originalFetch(`/__live/async/sse/${this.issued.transport}`, {
        headers: {
          Accept: "text/event-stream",
          Authorization: REFERENCE_AUTHORIZATION,
        },
        signal: this.abort.signal,
      });
      if (!response.ok || response.body === null) {
        boundedPush(observations.transportFailures, `sse:${response.status}`);
        throw new Error("reference_sse_open_failed");
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder("utf-8", { fatal: true });
      let buffered = "";
      for (;;) {
        const item = await reader.read();
        if (this.closed) return;
        if (item.done) {
          boundedPush(observations.transportFailures, "sse:eof");
          this.request.failed("transport_lost");
          return;
        }
        buffered += decoder.decode(item.value, { stream: true });
        for (;;) {
          const end = buffered.indexOf("\n\n");
          if (end < 0) break;
          const record = buffered.slice(0, end);
          buffered = buffered.slice(end + 2);
          const data = record
            .split("\n")
            .find((line) => line.startsWith("data:"))
            ?.slice(5);
          if (data !== undefined) deliverEnvelope(this, this.request, data);
        }
      }
    } catch {
      if (!this.abort.signal.aborted) boundedPush(observations.transportFailures, "sse:failed");
      if (!this.closed && !this.abort.signal.aborted) this.request.failed("transport_lost");
    }
  }
}

class ReferenceWebSocketPort {
  constructor(request) {
    this.closed = false;
    this.pending = new Map();
    this.request = request;
    this.issued = issuedByKind.get("websocket");
    this.nextControl = 0;
    if (this.issued === undefined) throw new Error("reference_websocket_transport_missing");
    if (this.issued.transport_generation !== request.transportGeneration) {
      throw new Error("reference_websocket_transport_generation_mismatch");
    }
    persistedRestart = false;
    const transportSequence = Number.parseInt(this.issued.transport.slice("transport-".length), 10);
    if (!Number.isSafeInteger(transportSequence) || transportSequence < 1) {
      throw new Error("reference_websocket_transport_invalid");
    }
    observations.portsCreated += 1;
    this.controlSequenceBase = BigInt(transportSequence) * 65_536n;
    this.socket = new WebSocket("/__live/async/ws", `suprnova-live-v1.${this.issued.transport}`);
    activePorts.add(this);
    this.socket.onopen = () => {
      if (!this.closed) request.opened();
    };
    this.socket.onmessage = (event) => {
      if (this.closed || typeof event.data !== "string") return;
      let value;
      try {
        value = JSON.parse(event.data);
      } catch {
        request.failed("protocol_invalid");
        return;
      }
      if (value.kind === "membership_authenticated") {
        const pending = this.pending.get(value.control_nonce);
        if (pending !== undefined) {
          this.pending.delete(value.control_nonce);
          pending.resolve(
            Object.freeze({
              descriptorBinding: value.descriptor_binding,
              kind: "authenticated",
              stream: value.stream,
              subscriptionId: value.subscription,
              transportGeneration: value.transport_generation,
            }),
          );
        }
        return;
      }
      if (value.kind !== "unsubscribed") deliverEnvelope(this, request, event.data);
    };
    this.socket.onerror = () => {
      if (!this.closed) request.failed("transport_lost");
    };
    this.socket.onclose = () => {
      if (!this.closed) request.failed("transport_lost");
    };
  }

  subscribe(subscription) {
    observations.subscriptionAttempts += 1;
    return new Promise((resolve) => {
      if (this.closed) {
        resolve(Object.freeze({ kind: "rejected", reason: "closed" }));
        return;
      }
      this.nextControl += 1;
      const controlNonce = (this.controlSequenceBase + BigInt(this.nextControl))
        .toString(36)
        .padStart(16, "0");
      this.pending.set(controlNonce, { resolve });
      this.socket.send(
        JSON.stringify({
          control_nonce: controlNonce,
          descriptor_binding: subscription.descriptorBinding,
          kind: "subscribe",
          stream: subscription.stream,
          subscription: subscription.subscriptionId,
          transport_generation: this.request.transportGeneration,
        }),
      );
    });
  }

  unsubscribe(subscriptionId) {
    if (!this.closed) {
      this.socket.send(JSON.stringify({ kind: "unsubscribe", subscription: subscriptionId }));
    }
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    for (const { resolve } of this.pending.values()) {
      resolve(Object.freeze({ kind: "rejected", reason: "closed" }));
    }
    this.pending.clear();
    this.socket.close(1000, "suprnova_live_async_closed");
    activePorts.delete(this);
    requestedGenerationByKind.set(
      "websocket",
      persistedRestart ? 1 : this.request.transportGeneration + 1,
    );
  }
}

const timers = Object.freeze({
  clearTimeout(handle) {
    if (controlledTimerRecords.delete(handle)) return;
    window.clearTimeout(handle);
  },
  timeout(callback, milliseconds) {
    if (controlledClock) {
      controlledTimerHandle += 1;
      controlledTimerRecords.set(controlledTimerHandle, { callback, milliseconds });
      return controlledTimerHandle;
    }
    return window.setTimeout(() => {
      callback();
    }, milliseconds);
  },
});

function fireControlledTimer(milliseconds) {
  for (const [handle, record] of controlledTimerRecords) {
    if (record.milliseconds !== milliseconds) continue;
    controlledTimerRecords.delete(handle);
    record.callback();
    return;
  }
  throw new Error("iteration_004_controlled_timer_missing");
}
const asyncOptions = Object.freeze({
  authority: Object.freeze({ authorize: issueAuthorization }),
  clock: Object.freeze({ now: () => 1_000 }),
  observeFreshness(observation) {
    boundedPush(observations.freshnessStates, observation.state);
  },
  randomness: Object.freeze({ number: () => 0.5 }),
  timers,
  transports: Object.freeze({
    eventSource(request) {
      return new ReferenceSsePort(request);
    },
    webSocket(request) {
      return new ReferenceWebSocketPort(request);
    },
  }),
});

function installScenarioControls() {
  document.querySelector("#remove-second-island")?.addEventListener("click", () => {
    document.querySelector('[data-suprnova-live-document-key="iteration-004-secondary"]')?.remove();
  });
}

function registerIncompatibleFeature(slot) {
  const surface = Reflect.get(window, FEATURE_SYMBOL);
  if (surface === undefined) throw new Error("iteration_004_feature_surface_missing");
  const feature = Object.freeze([
    Symbol.for("suprnova.live.feature.v1"),
    slot === "async" ? 1 : 0,
    99,
    0,
    Object.freeze({}),
    () => true,
  ]);
  boundedPush(observations.featureRegistrations, `${slot}:${surface.register(feature)}`);
}

if (syntheticLifecycle && !("onfreeze" in document)) {
  Object.defineProperty(document, "onfreeze", { configurable: true, value: null, writable: true });
}
if (syntheticLifecycle && !("onresume" in document)) {
  Object.defineProperty(document, "onresume", { configurable: true, value: null, writable: true });
}
installScenarioControls();
window.fetch = referenceUploadFetch;

if (format === "esm") {
  if (uploadArtifact === "current" && hasUploads) {
    const uploads = await import("/suprnova-live.uploads.esm.js");
    boundedPush(observations.featureRegistrations, `uploads:${uploads.uploadsRegistration}`);
    uploads.configureUploads({ chunkBytes: uploadChunkBytes, maxActive: 1, maxItems: 8 });
  }
  if (asyncArtifact === "current" && hasAsync) {
    const asynchronous = await import("/suprnova-live.async.esm.js");
    boundedPush(observations.featureRegistrations, `async:${asynchronous.asyncRegistration}`);
    asynchronous.configureAsync(asyncOptions);
  }
  if (uploadArtifact === "incompatible" && hasUploads) registerIncompatibleFeature("uploads");
  if (asyncArtifact === "incompatible" && hasAsync) registerIncompatibleFeature("async");
  const core = await import("/suprnova-live.esm.js");
  runtime = core.boot();
} else {
  const surface = Reflect.get(window, FEATURE_SYMBOL);
  const registrationProbe = Reflect.get(window, "__suprnovaIteration004ClassicRegistrationProbe");
  registrationProbe?.restore();
  for (const registration of registrationProbe?.registrations ?? []) {
    boundedPush(observations.featureRegistrations, registration);
  }
  for (const registration of Reflect.get(window, "__suprnovaIteration004Incompatible") ?? []) {
    boundedPush(observations.featureRegistrations, registration);
  }
  if (asyncArtifact === "current" && hasAsync) surface.configureAsync(asyncOptions);
  runtime = window.SuprnovaLive.boot();
}

Reflect.set(
  window,
  "__suprnovaIteration004",
  Object.freeze({
    advanceUploadClock(milliseconds) {
      if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) {
        throw new Error("iteration_004_upload_clock_step_invalid");
      }
      uploadClockNow += milliseconds;
    },
    advanceTransportReconnect() {
      fireControlledTimer(125);
    },
    async finalizeSelectedUpload() {
      if (readyUpload === null || readyUpload.readyRevision === null) {
        throw new Error("iteration_004_ready_upload_missing");
      }
      const response = await originalFetch("/scenario/iteration004/finalize-upload", {
        body: JSON.stringify({
          handle: readyUpload.handle,
          ready_revision: readyUpload.readyRevision,
        }),
        headers: {
          Authorization: REFERENCE_AUTHORIZATION,
          "Content-Type": "application/json",
        },
        method: "POST",
      });
      if (!response.ok) throw new Error("iteration_004_finalize_failed");
      const value = await response.json();
      readyUpload = null;
      return value.state;
    },
    freeze() {
      pauseNextUploadChunk = false;
      pauseEveryUploadChunk = false;
      uploadPauseGeneration = null;
      document.dispatchEvent(new Event("freeze"));
    },
    freshnessStates() {
      return [...observations.freshnessStates];
    },
    holdNextEnvelope() {
      holdNextEnvelope = true;
    },
    pauseNextUpload() {
      if (uploadPauseGeneration !== null || pauseNextUploadChunk || pauseEveryUploadChunk) {
        throw new Error("iteration_004_upload_pause_in_use");
      }
      pauseNextUploadChunk = true;
    },
    pauseEveryUploadChunk() {
      if (uploadPauseGeneration !== null || pauseNextUploadChunk || pauseEveryUploadChunk) {
        throw new Error("iteration_004_upload_pause_in_use");
      }
      pauseEveryUploadChunk = true;
    },
    async emitNextEnvelope() {
      const kind = transportKind === "websocket" ? "websocket" : "sse";
      const issued = issuedByKind.get(kind);
      if (issued === undefined) throw new Error("iteration_004_transport_missing");
      const response = await originalFetch("/__test/iteration-004/control/async/emit", {
        body: JSON.stringify({
          transport: issued.transport,
          transport_generation: issued.transport_generation,
        }),
        headers: { "Content-Type": "application/json" },
        method: "POST",
      });
      if (!response.ok) throw new Error("iteration_004_emit_failed");
    },
    releaseRetiredEnvelopes() {
      for (const held of heldEnvelopes.splice(0)) {
        observations.retiredEnvelopeAttempts += 1;
        held.request.message(held.encoded);
      }
    },
    resume() {
      const kind = transportKind === "websocket" ? "websocket" : "sse";
      requestedGenerationByKind.set(kind, 1);
      document.dispatchEvent(new Event("resume"));
    },
    async resumePausedUpload() {
      if (!Number.isSafeInteger(uploadPauseGeneration)) {
        throw new Error("iteration_004_upload_pause_missing");
      }
      const generation = uploadPauseGeneration;
      uploadPauseGeneration = null;
      const response = await originalFetch("/__test/iteration-004/control/upload/resume-chunk", {
        body: JSON.stringify({ pause_generation: generation }),
        headers: { "Content-Type": "application/json" },
        method: "POST",
      });
      if (!response.ok) throw new Error("iteration_004_upload_resume_failed");
    },
    async shutdown() {
      runtime.stop();
      await Promise.resolve();
    },
    async snapshot() {
      return structuredClone({
        ...observations,
        controlledTimerDelays: [...controlledTimerRecords.values()].map(
          ({ milliseconds }) => milliseconds,
        ),
        host: await hostInspection(),
        runtimeResources: runtimeResourceCounts(),
      });
    },
  }),
);
document.documentElement.setAttribute("data-iteration-004-ready", "true");
