export class UrlReflectionError extends Error {
  constructor(readonly code: "target" | "origin" | "path") {
    super(`url_reflection_${code}`);
    this.name = "UrlReflectionError";
  }
}

export function reflectedUrl(current: URL, target: string): URL {
  let candidate: URL;
  try {
    candidate = new URL(target, current);
  } catch {
    throw new UrlReflectionError("target");
  }
  if (candidate.username.length > 0 || candidate.password.length > 0) {
    throw new UrlReflectionError("target");
  }
  if (candidate.origin !== current.origin) throw new UrlReflectionError("origin");
  if (candidate.pathname !== current.pathname) throw new UrlReflectionError("path");
  try {
    return nativeNavigationIntent({
      base: current,
      target,
      method: "GET",
      history: "replace_query",
      prefetch: "none",
      transitionName: null,
      source: "reflection",
    }).target;
  } catch {
    throw new UrlReflectionError("target");
  }
}

export function applyUrlReflection(
  current: URL,
  target: string,
  replace: (target: URL) => void,
): URL {
  const candidate = reflectedUrl(current, target);
  replace(candidate);
  return candidate;
}
import { nativeNavigationIntent } from "../navigation/eligibility.js";
