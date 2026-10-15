/* global document, HTMLInputElement, HTMLElement, window */

const AUTHORIZATION = "Bearer task1-reference-session";
const originalFetch = window.fetch.bind(window);
let pauseGeneration = null;
let readyUpload = null;

function grant(headers) {
  const authorization = headers.get("Authorization") ?? "";
  const prefix = "SuprnovaUpload ";
  if (!authorization.startsWith(prefix) || authorization.length === prefix.length) {
    throw new Error("fresh_render_upload_grant_missing");
  }
  return authorization.slice(prefix.length);
}

function mappedUploadResponse(operation, value) {
  const response = {
    revision: String(value.revision),
    state: value.state === "created" ? "queued" : value.state,
  };
  if (operation === "create") {
    response.grant = value.grant;
    response.handle = value.handle;
  }
  if (operation === "status") response.nextChunkIndex = value.next_part;
  return response;
}

async function pauseFirstChunk(handle, uploadRevision) {
  const response = await originalFetch("/__test/iteration-004/control/upload/pause-chunk", {
    body: JSON.stringify({ handle, upload_revision: uploadRevision }),
    headers: { "Content-Type": "application/json" },
    method: "POST",
  });
  if (!response.ok) throw new Error("fresh_render_upload_pause_failed");
  const value = await response.json();
  pauseGeneration = value.pause_generation;
}

async function uploadFetch(input, init = {}) {
  const source = input instanceof Request ? input.url : String(input);
  const url = new URL(source, document.baseURI);
  if (url.origin !== window.location.origin || url.pathname !== "/__live/upload") {
    return originalFetch(input, init);
  }
  const headers = new Headers(init.headers);
  let operation = headers.get("X-Suprnova-Upload-Operation");
  let body = null;
  if (operation === null) {
    if (typeof init.body !== "string") throw new Error("fresh_render_upload_control_invalid");
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
      headers: { Authorization: AUTHORIZATION, "Content-Type": "application/json" },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "put_chunk") {
    const handle = headers.get("X-Suprnova-Upload-Handle");
    const part = headers.get("X-Suprnova-Upload-Chunk");
    response = await originalFetch(`/__live/uploads/${handle}/chunks/${part}`, {
      body: init.body,
      headers: {
        Authorization: AUTHORIZATION,
        "Content-Type": "application/octet-stream",
        "X-Live-Chunk-Bytes": String(init.body.byteLength),
        "X-Live-Chunk-Sha256": headers.get("X-Suprnova-Upload-Checksum") ?? "",
        "X-Live-Upload-Grant": grant(headers),
      },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "complete") {
    response = await originalFetch(`/__live/uploads/${body.handle}/complete`, {
      body: JSON.stringify({ grant: grant(headers) }),
      headers: { Authorization: AUTHORIZATION, "Content-Type": "application/json" },
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "cancel") {
    response = await originalFetch(`/__live/uploads/${body.handle}/cancel`, {
      headers: {
        Authorization: AUTHORIZATION,
        "X-Live-Upload-Grant": grant(headers),
      },
      keepalive: init.keepalive === true,
      method: "POST",
      signal: init.signal,
    });
  } else if (operation === "status") {
    response = await originalFetch(`/__live/uploads/${body.handle}`, {
      headers: {
        Authorization: AUTHORIZATION,
        "X-Live-Upload-Grant": grant(headers),
      },
      method: "GET",
      signal: init.signal,
    });
  } else {
    throw new Error("fresh_render_upload_operation_invalid");
  }
  if (!response.ok) return response;
  const value = await response.json();
  if (operation === "create") {
    readyUpload = { handle: value.handle, readyRevision: null };
    await pauseFirstChunk(value.handle, value.revision);
  } else if (operation === "complete" && readyUpload?.handle === body.handle) {
    readyUpload.readyRevision = value.revision;
  } else if (operation === "cancel" && readyUpload?.handle === body.handle) {
    pauseGeneration = null;
    readyUpload = null;
  }
  return new Response(JSON.stringify(mappedUploadResponse(operation, value)), {
    headers: { "Content-Type": "application/json" },
    status: response.status,
  });
}

window.fetch = uploadFetch;
const issued = await originalFetch("/__live/async/transports", {
  body: JSON.stringify({
    kind: "sse",
    position: null,
    prior_subscription: null,
    subscription: "orders",
    transport_generation: 1,
  }),
  headers: { Authorization: AUTHORIZATION, "Content-Type": "application/json" },
  method: "POST",
}).then((response) => response.json());
const membership = issued.memberships[0];

const uploads = await import("/suprnova-live.uploads.esm.js");
uploads.configureUploads({ chunkBytes: 256 * 1024, maxActive: 1, maxItems: 8 });
const asynchronous = await import("/suprnova-live.async.esm.js");
asynchronous.configureAsync({
  clock: { now: () => 1_000 },
  randomness: { number: () => 0.5 },
  timers: {
    clearTimeout: (handle) => window.clearTimeout(handle),
    // suprnova-correctness-delay-allow: product-timer -- reference host supplies the runtime's observable reconnect scheduling port
    timeout: (callback, milliseconds) => window.setTimeout(callback, milliseconds),
  },
});
const core = await import("/suprnova-live.esm.js");

const initialIsland = document.querySelector("[data-suprnova-live-island]");
const initialPreserved = document.querySelector("#fresh-render-preserved");
const initialReplacement = document.querySelector("#fresh-render-replacement-old");
const initialUploadInput = document.querySelector("#attachment-input");
const initialUploadProgress = document.querySelector("#attachment-progress");
if (
  !(initialPreserved instanceof HTMLElement) ||
  !(initialUploadInput instanceof HTMLInputElement)
) {
  throw new Error("fresh_render_initial_controls_missing");
}
initialPreserved.focus();
const evidence = {
  acceptedRevision: null,
  initialIsland,
  initialPreserved,
  initialReplacement,
  initialUploadInput,
  initialUploadProgress,
  requests: 0,
  async finalizeSelectedUpload() {
    if (readyUpload === null || readyUpload.readyRevision === null) {
      throw new Error("fresh_render_ready_upload_missing");
    }
    const response = await originalFetch("/scenario/iteration004/finalize-upload", {
      body: JSON.stringify({
        handle: readyUpload.handle,
        ready_revision: readyUpload.readyRevision,
      }),
      headers: { Authorization: AUTHORIZATION, "Content-Type": "application/json" },
      method: "POST",
    });
    if (!response.ok) throw new Error("fresh_render_finalize_failed");
    const value = await response.json();
    readyUpload = null;
    return value.state;
  },
  async resumePausedUpload() {
    if (!Number.isSafeInteger(pauseGeneration)) {
      throw new Error("fresh_render_upload_pause_missing");
    }
    const generation = pauseGeneration;
    pauseGeneration = null;
    const response = await originalFetch("/__test/iteration-004/control/upload/resume-chunk", {
      body: JSON.stringify({ pause_generation: generation }),
      headers: { "Content-Type": "application/json" },
      method: "POST",
    });
    if (!response.ok) throw new Error("fresh_render_upload_resume_failed");
  },
};
Object.defineProperty(window, "__suprnovaFreshRender", { value: evidence });

core.boot({
  diagnostics: "verbose",
  transport: {
    async fetch(input, init) {
      const headers = new Headers(init?.headers);
      headers.set("Authorization", AUTHORIZATION);
      headers.set("X-Live-Subscription", membership.subscription);
      headers.set("X-Live-Subscription-Authority", membership.authority);
      evidence.requests += 1;
      const response = await originalFetch(input, { ...init, headers });
      if (response.ok) {
        const value = await response.clone().json();
        evidence.acceptedRevision = value.accepted_revision;
      }
      return response;
    },
  },
});
document.documentElement.setAttribute("data-reference-fresh-render-ready", "true");
