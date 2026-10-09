// Where the reserved Live routes sit for the current page.
//
// The server names one of them, the action endpoint, in the page's
// configuration element, and the others sit beside it under `/__live/`.
// Behind a reverse proxy that serves the application under a path prefix,
// the server writes that endpoint under the prefix, so the upload and
// asynchronous routes follow it there (PFX-006). Without a configuration
// element, or with an endpoint that is not the reserved action route, the
// routes sit at the host root, where they always sat.

/**
 * The id of the configuration element, the same value as
 * `runtime/config.ts`'s `CONFIG_ELEMENT_ID` (a unit test holds the two
 * equal). It is written out here rather than imported so the upload and
 * asynchronous bundles do not carry the core's configuration reader.
 */
export const RESERVED_ROUTES_CONFIG_ELEMENT_ID = "suprnova-live-config";

const ACTION_ROUTE = "/__live/action";

/**
 * The root-relative path of the reserved Live route `route`, such as
 * `upload` or `async/subscriptions`: `/billing/__live/upload` on a page whose
 * configured endpoint is `/billing/__live/action`.
 */
export function reservedRoutePath(route: string, page?: Document): string {
  return `${reservedRouteRoot(page)}/__live/${route}`;
}

/**
 * The path in front of `/__live/action` in the page's configured endpoint,
 * or the empty string. The configuration element is the server's own
 * markup, which the core runtime already validated before any feature ran;
 * anything this reader cannot read leaves the routes at the host root.
 */
function reservedRouteRoot(page: Document | undefined): string {
  const documentRef = page ?? (typeof document === "undefined" ? undefined : document);
  if (documentRef === undefined) return "";
  try {
    const text = documentRef.getElementById(RESERVED_ROUTES_CONFIG_ELEMENT_ID)?.textContent ?? null;
    if (text === null) return "";
    const config: unknown = JSON.parse(text);
    if (typeof config !== "object" || config === null) return "";
    const endpoint: unknown = Reflect.get(config, "endpoint");
    if (typeof endpoint !== "string") return "";
    const path = new URL(endpoint, documentRef.baseURI).pathname;
    return path.endsWith(ACTION_ROUTE) ? path.slice(0, -ACTION_ROUTE.length) : "";
  } catch {
    return "";
  }
}
