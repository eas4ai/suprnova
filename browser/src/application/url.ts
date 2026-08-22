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
  if (candidate.origin !== current.origin) throw new UrlReflectionError("origin");
  if (candidate.pathname !== current.pathname) throw new UrlReflectionError("path");
  return new URL(candidate.href);
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
