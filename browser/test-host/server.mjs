import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";

import { buildRuntimeAssets } from "../scripts/build.mjs";
import { scenarios } from "./scenarios.mjs";

const host = "127.0.0.1";
const port = 4173;
const browserRoot = new URL("../", import.meta.url);
const dist = new URL("dist/", browserRoot);
await buildRuntimeAssets(dist.pathname);
const liveAttempts = new Map();

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

function liveResponse(parsed) {
  if (parsed.protocol_version === 2) {
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
      const key = `${target.searchParams.get("mode") ?? "normal"}:${parsed.correlation_id}`;
      const attempt = (liveAttempts.get(key) ?? 0) + 1;
      liveAttempts.set(key, attempt);
      if (target.searchParams.get("mode") === "retry" && attempt === 1) {
        respond(response, 503, "", { "content-type": mediaType });
        return;
      }
      respond(response, 200, liveResponse(parsed), { "content-type": mediaType });
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
