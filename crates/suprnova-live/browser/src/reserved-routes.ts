// Where the reserved Live routes sit for the current page.
//
// The server names one of them, the action endpoint, in the page's
// configuration element, and the others sit beside it under `/__live/`.
// Behind a reverse proxy that serves the application under a path prefix,
// the server writes that endpoint under the prefix, so the upload and
// asynchronous routes follow it there (PFX-006). A standalone Inertia page
// has no Live configuration element, so its root comes from explicit server
// markup instead. Anything neither source can name safely leaves the routes
// at the host root, where they always sat.

/**
 * The id of the configuration element, the same value as
 * `runtime/config.ts`'s `CONFIG_ELEMENT_ID` (a unit test holds the two
 * equal). It is written out here rather than imported so the upload and
 * asynchronous bundles do not carry the core's configuration reader.
 */
export const RESERVED_ROUTES_CONFIG_ELEMENT_ID = "suprnova-live-config";

const ACTION_ROUTE = "/__live/action";
const ROOT_META_SELECTOR = 'meta[name="suprnova-live-root"]';

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
 * the standalone page's explicit public root, or the empty string. A valid
 * Live endpoint takes precedence because it is the route the running core
 * has already selected.
 */
function reservedRouteRoot(page: Document | undefined): string {
  const documentRef = page ?? (typeof document === "undefined" ? undefined : document);
  if (documentRef === undefined) return "";
  return configuredRouteRoot(documentRef) ?? explicitRouteRoot(documentRef) ?? "";
}

function configuredRouteRoot(documentRef: Document): string | null {
  try {
    const text = documentRef.getElementById(RESERVED_ROUTES_CONFIG_ELEMENT_ID)?.textContent ?? null;
    if (text === null) return null;
    const config: unknown = JSON.parse(text);
    if (typeof config !== "object" || config === null) return null;
    const endpoint: unknown = Reflect.get(config, "endpoint");
    if (typeof endpoint !== "string") return null;
    const path = new URL(endpoint, documentRef.baseURI).pathname;
    return path.endsWith(ACTION_ROUTE) ? path.slice(0, -ACTION_ROUTE.length) : null;
  } catch {
    return null;
  }
}

function explicitRouteRoot(documentRef: Document): string | null {
  try {
    const value = documentRef.querySelector<HTMLMetaElement>(ROOT_META_SELECTOR)?.content;
    if (value === undefined) return null;
    if (value === "" || value === "/") return "";
    const rest = value.startsWith("/") ? value.slice(1) : null;
    if (rest === null || !/^[-A-Za-z0-9._~/]+$/u.test(value)) return null;
    return rest
      .split("/")
      .every((segment) => segment.length !== 0 && segment !== "." && segment !== "..")
      ? value
      : null;
  } catch {
    return null;
  }
}
