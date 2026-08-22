import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";

import { buildRuntimeAssets } from "../scripts/build.mjs";
import {
  continuityBody,
  morphChild,
  preservationBody,
  scenarios,
  stimulusChild,
  transitionBody,
} from "./scenarios.mjs";

const host = "127.0.0.1";
const port = 4173;
const browserRoot = new URL("../", import.meta.url);
const dist = new URL("dist/", browserRoot);
await buildRuntimeAssets(dist.pathname);
const liveAttempts = new Map();

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function requestBody(request, maximum = 1_048_576) {
  const chunks = [];
  let total = 0;
  for await (const chunk of request) {
    total += chunk.length;
    if (total > maximum) throw new Error("request_too_large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

function liveResponse(parsed, mode) {
  if (parsed.protocol_version === 2) {
    if (mode === "redirect") {
      return JSON.stringify({
        child_deliveries: [],
        correlation_id: parsed.correlation_id,
        effects: [],
        events: [],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        redirect: "/transport-accepted",
        url_intent: null,
        validation: {},
      });
    }
    if (
      (mode === "morph-unsafe" || mode === "teleport-late-target") &&
      parsed.operations?.some((operation) => operation.kind === "fresh_render")
    ) {
      return JSON.stringify({
        child_deliveries: [],
        correlation_id: parsed.correlation_id,
        effects: [],
        events: [],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        url_intent: {
          kind: "navigated",
          target: mode === "morph-unsafe" ? "/morph-recovered" : "/teleport-target-rejected",
        },
        validation: {},
      });
    }
    if (mode === "navigated") {
      return JSON.stringify({
        child_deliveries: [],
        correlation_id: parsed.correlation_id,
        effects: [],
        events: [],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        url_intent: { kind: "navigated", target: "/response-order-target?kind=navigated" },
        validation: {},
      });
    }
    if (
      mode === "committed" ||
      mode === "no-render" ||
      mode === "morph-identity" ||
      mode === "stimulus-morph" ||
      mode === "morph-unsafe" ||
      mode === "preservation" ||
      mode === "continuity" ||
      mode === "transitions" ||
      mode === "recovery-fails" ||
      mode === "teleport-late-target"
    ) {
      const revision = String(BigInt(parsed.base_revision) + 1n);
      const snapshot = structuredClone(parsed.snapshot.envelope);
      snapshot.body.form = "instance";
      snapshot.body.revision = revision;
      const encoded = Buffer.from(JSON.stringify(snapshot), "utf8").toString("base64url");
      const instance = snapshot.body.instance_id;
      const documentKey = parsed.extensions.x_suprnova_live_document_key_v1;
      const body =
        mode === "morph-identity"
          ? `<button id="morph-action" live:click.prevent="save">Morph</button>
              <ol id="morph-list">
                <li id="beta" data-suprnova-live-key="beta">Beta updated</li>
                <li id="alpha" data-suprnova-live-key="alpha">Alpha updated</li>
                <li id="new" data-suprnova-live-key="new">New</li>
              </ol>
              ${morphChild("Nested server mutation")}`
          : mode === "stimulus-morph"
            ? `<button id="stimulus-action" live:click.prevent="save">Morph</button>
                <div id="stimulus-preserved" data-controller="probe" data-probe="preserved" data-suprnova-live-key="preserved" data-state="morphed"></div>
                <div id="stimulus-inserted" data-controller="probe" data-probe="inserted" data-suprnova-live-key="inserted"></div>
                <div id="stimulus-detached" data-controller="probe" data-probe="detached" data-suprnova-live-key="detached"></div>
                  ${stimulusChild()}`
            : mode === "preservation"
              ? preservationBody(revision)
              : mode === "continuity"
                ? continuityBody(revision)
                : mode === "transitions"
                  ? transitionBody(revision)
                  : mode === "recovery-fails"
                    ? '<p id="recovery-corrupt">Unsafe recovery</p><script>document.documentElement.dataset.recoveryScriptExecuted = "true";</script>'
                    : mode === "teleport-late-target"
                      ? '<button id="late-teleport-action" live:click.prevent="save">Attempt teleport</button><div id="late-teleported" data-suprnova-live-key="late-teleported" live:teleport="#late-modal-root">Late teleport</div>'
                      : mode === "morph-unsafe"
                        ? '<p id="morph-unsafe-content">Unsafe replacement</p><script>document.documentElement.dataset.morphScriptExecuted = "true";</script>'
                        : '<p id="response-content">Updated</p>';
      const rootId = mode === "stimulus-morph" ? ' id="stimulus-island"' : "";
      const html = `<section data-suprnova-live-root="search-results" data-suprnova-live-island data-suprnova-live-component="catalog.search" data-suprnova-live-slot="search-results" data-suprnova-live-document-key="${documentKey}" data-suprnova-live-protocol-min="2" data-suprnova-live-contract="1" data-suprnova-live-snapshot-kind="instance" data-suprnova-live-snapshot="${encoded}" data-suprnova-live-revision="${revision}" data-suprnova-live-lazy-complete="false" data-suprnova-live-instance="${instance}"${rootId}>${body}</section>`;
      return JSON.stringify({
        accepted_revision: revision,
        child_deliveries: [],
        correlation_id: parsed.correlation_id,
        effects: [{ name: "probe", payload: {} }],
        events: [{ name: "saved", payload: {} }],
        extensions: {},
        outcome: "accepted",
        protocol_version: 2,
        render: mode === "no-render" ? { kind: "no_render" } : { html, kind: "html" },
        snapshot,
        url_intent:
          mode === "committed"
            ? {
                kind: "reflected",
                target: "/scenario/responseCommitted?state=done",
              }
            : null,
        validation: {},
      });
    }
    return JSON.stringify({
      child_deliveries: [],
      correlation_id: parsed.correlation_id,
      effects: [],
      error: { category: "internal", detail: "operation_rejected", recovery: "retain_dom" },
      events: [],
      extensions: {},
      outcome: "rejected",
      protocol_version: 2,
      url_intent: null,
      validation: {},
    });
  }
  if (mode === "redirect") {
    return JSON.stringify({
      correlation_id: parsed.correlation_id,
      effects: [],
      events: [],
      extensions: {},
      outcome: "accepted",
      protocol_version: 1,
      redirect: "/transport-accepted",
      validation: {},
    });
  }
  return JSON.stringify({
    correlation_id: parsed.correlation_id,
    effects: [],
    error: { category: "internal", detail: "operation_rejected", recovery: "retain_dom" },
    events: [],
    extensions: {},
    outcome: "rejected",
    protocol_version: 1,
    validation: {},
  });
}

function respond(response, status, body, headers = {}) {
  response.writeHead(status, {
    "cache-control": "no-store",
    "content-type": "text/plain; charset=utf-8",
    "x-suprnova-conformance-host": "1",
    ...headers,
  });
  response.end(body);
}

const server = createServer(async (request, response) => {
  const target = new URL(request.url ?? "/", `http://${host}:${port}`);
  if (target.pathname === "/health") {
    respond(response, 200, "ok");
    return;
  }
  if (target.pathname === "/live") {
    try {
      if (request.method !== "POST") throw new Error("method_not_allowed");
      const mediaType = request.headers["content-type"];
      if (typeof mediaType !== "string") throw new Error("media_type_missing");
      const parsed = JSON.parse(await requestBody(request));
      const mode = target.searchParams.get("mode") ?? "normal";
      const key = `${mode}:${parsed.correlation_id}`;
      const attempt = (liveAttempts.get(key) ?? 0) + 1;
      liveAttempts.set(key, attempt);
      if (mode === "retry" && attempt === 1) {
        respond(response, 503, "", { "content-type": mediaType });
        return;
      }
      if (mode === "normal" || mode === "retry") await delay(3_000);
      respond(response, 200, liveResponse(parsed, mode), {
        "content-type": mediaType,
      });
    } catch {
      respond(response, 400, "invalid live conformance request");
    }
    return;
  }
  if (target.pathname.startsWith("/scenario/")) {
    const name = target.pathname.slice("/scenario/".length);
    const scenario = scenarios[name];
    if (scenario === undefined) {
      respond(response, 404, "unknown scenario");
      return;
    }
    respond(response, 200, scenario.html, {
      "content-type": "text/html; charset=utf-8",
      ...(scenario.headers ?? {}),
    });
    return;
  }
  if (target.pathname.startsWith("/assets/")) {
    const file = target.pathname.slice("/assets/".length);
    if (
      !["suprnova-live.classic.js", "suprnova-live.esm.js", "suprnova-live.assets.json"].includes(
        file,
      )
    ) {
      respond(response, 404, "unknown asset");
      return;
    }
    try {
      const body = await readFile(join(dist.pathname, file));
      const contentType = extname(file) === ".json" ? "application/json" : "text/javascript";
      respond(response, 200, body, {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": `${contentType}; charset=utf-8`,
      });
    } catch {
      respond(response, 404, "asset unavailable");
    }
    return;
  }
  if (target.pathname === "/test-vendor/stimulus.js") {
    try {
      const body = await readFile(
        new URL("node_modules/@hotwired/stimulus/dist/stimulus.js", browserRoot),
      );
      respond(response, 200, body, { "content-type": "text/javascript; charset=utf-8" });
    } catch {
      respond(response, 404, "test vendor unavailable");
    }
    return;
  }
  respond(response, 404, "not found");
});

server.listen(port, host);

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
