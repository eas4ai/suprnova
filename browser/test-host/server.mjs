import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";

import { buildRuntimeAssets } from "../scripts/build.mjs";
import {
  continuityBody,
  externalClassicBootSource,
  externalModuleBootSource,
  morphChild,
  optionalDriverSource,
  preservationBody,
  scenarios,
  stimulusChild,
  transitionBody,
  uploadBody,
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
      mode === "full-flow" ||
      mode === "no-render" ||
      mode === "morph-identity" ||
      mode === "stimulus-morph" ||
      mode === "morph-unsafe" ||
      mode === "preservation" ||
      mode === "continuity" ||
      mode === "uploads-morph" ||
      mode === "transitions" ||
      mode === "hostile-extreme-morph" ||
      mode === "hostile-duplicate-identity" ||
      mode === "lifecycle" ||
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
                : mode === "uploads-morph"
                  ? uploadBody(revision !== "8")
                  : mode === "transitions"
                    ? transitionBody(revision)
                    : mode === "hostile-extreme-morph"
                      ? `<button id="hostile-action" live:click.prevent="save">Exercise hostile response</button>${"<div>".repeat(129)}<p>Too deep</p>${"</div>".repeat(129)}`
                      : mode === "hostile-duplicate-identity"
                        ? '<button id="hostile-action" live:click.prevent="save">Exercise hostile response</button><div data-suprnova-live-key="duplicate">First</div><div data-suprnova-live-key="duplicate">Second</div>'
                        : mode === "recovery-fails"
                          ? '<p id="recovery-corrupt">Unsafe recovery</p><script>document.documentElement.dataset.recoveryScriptExecuted = "true";</script>'
                          : mode === "teleport-late-target"
                            ? '<button id="late-teleport-action" live:click.prevent="save">Attempt teleport</button><div id="late-teleported" data-suprnova-live-key="late-teleported" live:teleport="#late-modal-root">Late teleport</div>'
                            : mode === "morph-unsafe"
                              ? '<p id="morph-unsafe-content" onclick="document.documentElement.dataset.morphHandlerExecuted = \'true\'">Unsafe replacement</p><script>document.documentElement.dataset.morphScriptExecuted = "true";</script>'
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
            : mode === "full-flow"
              ? {
                  kind: "reflected",
                  target: "/scenario/fullFlow?state=done",
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
      if (mode === "lifecycle") await delay(250);
      if (mode === "hostile-malformed-utf8") {
        respond(response, 200, Buffer.from([0xff, 0xfe, 0xfd]), {
          "content-type": mediaType,
        });
        return;
      }
      if (mode === "hostile-huge-json") {
        respond(response, 200, JSON.stringify({ padding: "x".repeat(1_100_000) }), {
          "content-type": mediaType,
        });
        return;
      }
      if (mode === "hostile-prototype-key") {
        respond(response, 200, '{"__proto__":{"polluted":true}}', {
          "content-type": mediaType,
        });
        return;
      }
      respond(response, 200, liveResponse(parsed, mode), {
        "content-type": mediaType,
      });
    } catch {
      respond(response, 400, "invalid live conformance request");
    }
    return;
  }
  if (target.pathname === "/navigation/redirect") {
    response.writeHead(302, { location: "/scenario/navigationDestination?redirected=1" });
    response.end();
    return;
  }
  if (target.pathname === "/test-boot/module.js") {
    respond(response, 200, externalModuleBootSource, {
      "cache-control": "public, max-age=31536000, immutable",
      "content-type": "text/javascript; charset=utf-8",
    });
    return;
  }
  if (target.pathname === "/test-boot/classic.js") {
    respond(response, 200, externalClassicBootSource, {
      "cache-control": "public, max-age=31536000, immutable",
      "content-type": "text/javascript; charset=utf-8",
    });
    return;
  }
  if (target.pathname === "/test-boot/features.js") {
    respond(response, 200, optionalDriverSource, {
      "cache-control": "public, max-age=31536000, immutable",
      "content-type": "text/javascript; charset=utf-8",
    });
    return;
  }
  if (target.pathname === "/test-async/lifecycle.js") {
    try {
      const body = await readFile(new URL("test-host/async-lifecycle.mjs", browserRoot));
      respond(response, 200, body, {
        "access-control-allow-origin": "http://127.0.0.1:4174",
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      });
    } catch {
      respond(response, 404, "async lifecycle asset unavailable");
    }
    return;
  }
  if (target.pathname === "/scenario/iteration004-driver.js") {
    try {
      const body = await readFile(new URL("test-host/iteration-004.mjs", browserRoot));
      respond(response, 200, body, {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      });
    } catch {
      respond(response, 404, "iteration 004 driver unavailable");
    }
    return;
  }
  if (target.pathname === "/scenario/reference-fresh-render-driver.js") {
    try {
      const body = await readFile(new URL("test-host/reference-fresh-render.mjs", browserRoot));
      respond(response, 200, body, {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      });
    } catch {
      respond(response, 404, "reference fresh-render driver unavailable");
    }
    return;
  }
  if (target.pathname === "/scenario/iteration004-incompatible-feature.js") {
    respond(
      response,
      200,
      `(() => {
        const slot = document.currentScript?.dataset.featureSlot;
        const surface = globalThis[Symbol.for("suprnova.live.features.v1")];
        const result = surface?.register(Object.freeze([Symbol.for("suprnova.live.feature.v1"), slot === "async" ? 1 : 0, 99, 0, Object.freeze({}), () => true]));
        globalThis.__suprnovaIteration004Incompatible ??= [];
        globalThis.__suprnovaIteration004Incompatible.push(slot + ":" + result);
      })();`,
      {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      },
    );
    return;
  }
  if (target.pathname === "/scenario/iteration004-classic-registration-probe.js") {
    respond(
      response,
      200,
      `(() => {
        const nativeApply = Reflect.apply;
        const registrations = [];
        Reflect.apply = function iteration004ObservedApply(target, thisArgument, argumentsList) {
          const result = nativeApply(target, thisArgument, argumentsList);
          const feature = argumentsList?.[0];
          const surface = globalThis[Symbol.for("suprnova.live.features.v1")];
          if (thisArgument === surface && Array.isArray(feature) && feature[0] === Symbol.for("suprnova.live.feature.v1") && (feature[1] === 0 || feature[1] === 1)) {
            registrations.push((feature[1] === 0 ? "uploads:" : "async:") + result);
          }
          return result;
        };
        globalThis.__suprnovaIteration004ClassicRegistrationProbe = Object.freeze({
          registrations,
          restore() { Reflect.apply = nativeApply; },
        });
      })();`,
      {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      },
    );
    return;
  }
  if (target.pathname === "/scenario/iteration004-axe.js") {
    try {
      const body = await readFile(new URL("node_modules/axe-core/axe.min.js", browserRoot));
      respond(response, 200, body, {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/javascript; charset=utf-8",
      });
    } catch {
      respond(response, 404, "iteration 004 axe asset unavailable");
    }
    return;
  }
  if (target.pathname === "/scenario/iteration004.css") {
    respond(
      response,
      200,
      `#iteration-upload-error { display: none; }
#iteration-upload-progress[data-live-upload-state="failed"] ~ #iteration-upload-error,
#iteration-upload-progress[data-live-upload-state="expired"] ~ #iteration-upload-error { display: block; }`,
      {
        "cache-control": "public, max-age=31536000, immutable",
        "content-type": "text/css; charset=utf-8",
      },
    );
    return;
  }
  if (target.pathname === "/navigation/download") {
    respond(response, 200, "downloaded report", {
      "content-disposition": 'attachment; filename="report.txt"',
      "content-type": "text/plain; charset=utf-8",
    });
    return;
  }
  if (target.pathname === "/navigation/post") {
    if (request.method !== "POST") {
      respond(response, 405, "method not allowed");
      return;
    }
    const body = await requestBody(request);
    respond(
      response,
      200,
      `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>POST destination</title></head><body><main><h1 id="post-focus" tabindex="-1">POST destination</h1><p id="post-body">${body.replaceAll("<", "&lt;")}</p></main></body></html>`,
      { "content-type": "text/html; charset=utf-8" },
    );
    return;
  }
  if (target.pathname.startsWith("/scenario/")) {
    const name = target.pathname.slice("/scenario/".length);
    const scenario = scenarios[name];
    if (scenario === undefined) {
      respond(response, 404, "unknown scenario");
      return;
    }
    const scenarioHeaders =
      typeof scenario.headers === "function" ? scenario.headers() : (scenario.headers ?? {});
    const scenarioHtml =
      typeof scenario.html === "function" ? scenario.html(target.searchParams) : scenario.html;
    respond(response, scenario.status ?? 200, scenarioHtml, {
      "content-type": "text/html; charset=utf-8",
      ...scenarioHeaders,
    });
    return;
  }
  if (target.pathname.startsWith("/assets/")) {
    const file = target.pathname.slice("/assets/".length);
    if (
      ![
        "suprnova-live.classic.js",
        "suprnova-live.esm.js",
        "suprnova-live.uploads.esm.js",
        "suprnova-live.async.esm.js",
        "suprnova-live.assets.json",
      ].includes(file)
    ) {
      respond(response, 404, "unknown asset");
      return;
    }
    try {
      const body = await readFile(join(dist.pathname, file));
      const contentType = extname(file) === ".json" ? "application/json" : "text/javascript";
      respond(response, 200, body, {
        "access-control-allow-origin": "http://127.0.0.1:4174",
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
  if (target.pathname === "/test-vendor/axe.js") {
    try {
      const body = await readFile(new URL("node_modules/axe-core/axe.min.js", browserRoot));
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
