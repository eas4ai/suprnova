/*
 * Deploy this ES module on a proxied hostname whose DNS points at the application.
 * Unpublished paths and mutations use fetch(request) to reach that application.
 * Example wrangler.toml:
 *
 * name = "suprnova-publication"
 * main = "cloudflare_worker.js"
 * compatibility_date = "2026-10-23"
 * routes = [{ pattern = "www.example.com/*", zone_name = "example.com" }]
 * [[r2_buckets]]
 * binding = "BUCKET"
 * bucket_name = "public-pages"
 * [vars]
 * ROOT_PREFIX = "" # Match S3Config.root, empty when none.
 * MANIFEST_TTL_SECONDS = "5" # Match CloudflareDriver.with_manifest_ttl.
 *
 * R2 get returns null for a missing key and an object body with httpMetadata,
 * customMetadata and the quoted httpEtag on a hit. writeHttpMetadata copies the
 * native delivery headers, including cacheExpiry as Expires:
 * https://developers.cloudflare.com/r2/api/workers/workers-api-reference/
 * The Cache API stores public responses under their unchanged public URLs.
 * Requests with a query bypass the cache so URL purges need no custom key:
 * https://developers.cloudflare.com/r2/examples/cache-api/
 * https://developers.cloudflare.com/workers/runtime-apis/cache/
 * Cached entries carry their selected object key internally so a new manifest
 * cannot reuse a previous revision. Application fallback responses are never cached.
 */

let cachedManifest = null;
const OBJECT_KEY_HEADER = "x-suprnova-r2-object-key";

function rootFor(rootPrefix) {
    const root = (rootPrefix || "").replace(/^\/+|\/+$/g, "");
    return root ? `${root}/` : "";
}

/** Maps public URLs to publication keys; null keeps internal control objects private. */
export function keyFor(pathname, live, rootPrefix) {
    if (pathname === "/_pub/manifest.json" || pathname === "/_pub/status.json") {
        return null;
    }
    const root = rootFor(rootPrefix);
    if (pathname.endsWith("/index.json")) {
        return `${root}_pub/${live}${pathname}`;
    }
    if (pathname.startsWith("/_pub/") || pathname.split("/").pop().includes(".")) {
        return root + pathname.slice(1);
    }
    const page = pathname.replace(/^\/+|\/+$/g, "");
    return `${root}_pub/${live}/${page ? `${page}/` : ""}index.html`;
}

/** Restores validated policy metadata while native metadata owns the representation. */
export function responseHeaders(object) {
    const headers = new Headers();
    const metadata = object.httpMetadata || {};
    for (const [field, name] of [
        ["contentType", "Content-Type"],
        ["cacheControl", "Cache-Control"],
        ["contentEncoding", "Content-Encoding"],
        ["contentLanguage", "Content-Language"],
        ["contentDisposition", "Content-Disposition"],
    ]) {
        if (metadata[field] !== undefined) headers.set(name, metadata[field]);
    }
    if (metadata.cacheExpiry !== undefined) {
        headers.set("Expires", new Date(metadata.cacheExpiry).toUTCString());
    }
    for (const [name, value] of Object.entries(object.customMetadata || {})) {
        if (!["content-type", "content-length", "etag", "content-encoding"].includes(name.toLowerCase())) {
            headers.set(name, value);
        }
    }
    headers.set("ETag", object.httpEtag);
    if ((headers.get("Content-Type") || "").startsWith("text/html") && !headers.has("Content-Security-Policy")) {
        headers.set("Content-Security-Policy", "default-src 'none'; sandbox");
    }
    return headers;
}

async function manifestFor(env) {
    const root = rootFor(env.ROOT_PREFIX);
    const now = Date.now();
    if (cachedManifest && cachedManifest.bucket === env.BUCKET && cachedManifest.root === root && now < cachedManifest.expires) {
        return cachedManifest.manifest;
    }
    let manifest = null;
    try {
        const object = await env.BUCKET.get(`${root}_pub/manifest.json`);
        const value = object ? await object.json() : null;
        if (value && typeof value.live === "string" && value.live && value.live.split("/").every(part => /^[A-Za-z0-9_.-]+$/.test(part) && part !== "." && part !== "..")) {
            // Index live routes once per refresh, rather than scanning the inventory on every hit.
            const pages = new Map();
            const objects = value.versions?.[value.live]?.objects;
            if (Array.isArray(objects)) {
                for (const entry of objects) {
                    if (!entry.asset) pages.set(entry.url, entry);
                }
            }
            manifest = { live: value.live, pages };
        }
    } catch {
        // A missing or unreadable manifest selects the application, never a stale revision.
    }
    const configured = Number(env.MANIFEST_TTL_SECONDS ?? 5);
    const ttl = Number.isFinite(configured) && configured >= 0 ? configured : 5;
    // Measure from the start of the read so slow I/O cannot extend a stale selection.
    cachedManifest = { bucket: env.BUCKET, root, manifest, expires: now + ttl * 1000 };
    return manifest;
}

async function objectResponse(request, env, context, key, entry) {
    const cache = new URL(request.url).search ? null : globalThis.caches?.default;
    const cacheKey = new Request(request.url, { method: "GET" });
    if (cache) {
        try {
            const hit = await cache.match(cacheKey);
            if (hit && hit.headers.get(OBJECT_KEY_HEADER) === key) return hit;
        } catch {
            console.warn("Publication cache read failed; reading R2");
        }
    }
    const object = await env.BUCKET.get(key);
    if (object === null) return null;
    const response = new Response(object.body, { status: 200, headers: responseHeaders(object) });
    if (cache && request.method === "GET") {
        const stored = response.clone();
        stored.headers.set(OBJECT_KEY_HEADER, key);
        const tags = entry?.dependency_tags;
        if (Array.isArray(tags) && tags.length) {
            stored.headers.append("Cache-Tag", tags.join(","));
        }
        const write = cache.put(cacheKey, stored).catch(() => {
            console.warn("Publication cache write failed; serving R2 response");
        });
        if (context) context.waitUntil(write);
        else await write;
    }
    return response;
}

export default {
    /** Serves published GET and HEAD objects while preserving the application for every miss. */
    async fetch(request, env, context) {
        if (request.method !== "GET" && request.method !== "HEAD") return fetch(request);
        const pathname = new URL(request.url).pathname;
        if (pathname === "/_pub/manifest.json" || pathname === "/_pub/status.json") return fetch(request);
        const manifest = await manifestFor(env);
        if (!manifest) return fetch(request);
        let key = keyFor(pathname, manifest.live, env.ROOT_PREFIX);
        // The current publisher records flat page stems. Its inventory is authoritative
        // when present, while keyFor also supports directory-style publication keys.
        const url = pathname.endsWith("/index.json") ? pathname : pathname.replace(/\/+$/, "") || "/";
        const versionPrefix = `${rootFor(env.ROOT_PREFIX)}_pub/${manifest.live}/`;
        const entry = key.startsWith(versionPrefix) ? manifest.pages.get(url) : undefined;
        if (entry && typeof entry.path === "string" && entry.path.startsWith(`_pub/${manifest.live}/`) && !entry.path.split("/").some(part => part === "." || part === "..")) {
            key = rootFor(env.ROOT_PREFIX) + entry.path;
        }
        const response = await objectResponse(request, env, context, key, entry);
        if (!response) return fetch(request);
        const headers = new Headers(response.headers);
        headers.delete(OBJECT_KEY_HEADER);
        const matches = (request.headers.get("If-None-Match") || "").split(",").some(value => {
            const tag = value.trim();
            return tag === "*" || tag.replace(/^W\//, "") === headers.get("ETag");
        });
        if (matches) return new Response(null, { status: 304, headers });
        return new Response(request.method === "HEAD" ? null : response.body, { status: 200, headers });
    },
};
