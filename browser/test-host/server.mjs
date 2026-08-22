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
    respond(response, 501, '{"error":"conformance_endpoint_not_implemented"}', {
      "content-type": "application/json; charset=utf-8",
    });
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
  respond(response, 404, "not found");
});

server.listen(port, host);

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
