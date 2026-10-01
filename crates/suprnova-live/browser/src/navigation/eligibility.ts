export interface NativeNavigationIntent {
  readonly target: URL;
  readonly method: "GET" | "HEAD" | "POST";
  readonly history: "navigate" | "replace_query";
  readonly prefetch: "none" | "link" | "speculation";
  readonly transitionName: string | null;
}

export type NativeNavigationSource = "anchor" | "form" | "redirect" | "refresh" | "reflection";

export interface NavigationActivation {
  readonly button: number;
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
}

export interface NativeNavigationCandidate {
  readonly base: URL;
  readonly target: string | URL;
  readonly method: "GET" | "HEAD" | "POST";
  readonly history: "navigate" | "replace_query";
  readonly prefetch: "none" | "link" | "speculation";
  readonly transitionName: string | null;
  readonly source: NativeNavigationSource;
  readonly activation?: NavigationActivation;
  readonly download?: boolean;
  readonly targetContext?: string | null;
  readonly response?: Readonly<{ status: number; mediaType: string }>;
}

export type NavigationEligibilityErrorCode =
  "target" | "scheme" | "credentials" | "method" | "history" | "transition_name";

export class NavigationEligibilityError extends Error {
  constructor(readonly code: NavigationEligibilityErrorCode) {
    super(`navigation_eligibility_error:${code}`);
    this.name = "NavigationEligibilityError";
  }
}

const TRANSITION_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/u;

function checkedTarget(candidate: NativeNavigationCandidate): URL {
  let target: URL;
  try {
    target = new URL(candidate.target, candidate.base);
  } catch {
    throw new NavigationEligibilityError("target");
  }
  if (target.protocol !== "http:" && target.protocol !== "https:") {
    throw new NavigationEligibilityError("scheme");
  }
  if (target.username.length > 0 || target.password.length > 0) {
    throw new NavigationEligibilityError("credentials");
  }
  return target;
}

function checkedTransitionName(name: string | null): string | null {
  if (name === null) return null;
  if (name === "none" || name.startsWith("--") || !TRANSITION_NAME.test(name)) {
    throw new NavigationEligibilityError("transition_name");
  }
  return name;
}

function modifiedActivation(activation: NavigationActivation | undefined): boolean {
  return (
    activation !== undefined &&
    (activation.button !== 0 ||
      activation.altKey ||
      activation.ctrlKey ||
      activation.metaKey ||
      activation.shiftKey)
  );
}

function sameDocumentFragment(base: URL, target: URL): boolean {
  return (
    base.origin === target.origin &&
    base.pathname === target.pathname &&
    base.search === target.search &&
    base.hash !== target.hash
  );
}

function unsafeEnhancementTarget(target: string | URL): boolean {
  if (target instanceof URL) return false;
  if (target.startsWith("//") || target.includes("\\")) return true;
  for (let index = 0; index < target.length; index += 1) {
    const code = target.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

function supportsDocumentEnhancement(candidate: NativeNavigationCandidate, target: URL): boolean {
  if (candidate.method !== "GET" && candidate.method !== "HEAD") return false;
  if (unsafeEnhancementTarget(candidate.target)) return false;
  if (candidate.download === true) return false;
  if (
    candidate.targetContext !== undefined &&
    candidate.targetContext !== null &&
    candidate.targetContext !== "" &&
    candidate.targetContext.toLowerCase() !== "_self"
  ) {
    return false;
  }
  if (modifiedActivation(candidate.activation)) return false;
  if (target.origin !== candidate.base.origin || sameDocumentFragment(candidate.base, target)) {
    return false;
  }
  if (candidate.response !== undefined) {
    const mediaType = candidate.response.mediaType.split(";", 1)[0]?.trim().toLowerCase();
    if (
      candidate.response.status < 200 ||
      candidate.response.status >= 400 ||
      mediaType !== "text/html"
    ) {
      return false;
    }
  }
  return true;
}

function validateReflection(candidate: NativeNavigationCandidate, target: URL): void {
  if (candidate.history !== "replace_query") return;
  if (
    unsafeEnhancementTarget(candidate.target) ||
    candidate.source !== "reflection" ||
    candidate.method !== "GET" ||
    target.origin !== candidate.base.origin ||
    target.pathname !== candidate.base.pathname
  ) {
    throw new NavigationEligibilityError("history");
  }
}

export function nativeNavigationIntent(
  candidate: NativeNavigationCandidate,
): NativeNavigationIntent {
  if (!(["GET", "HEAD", "POST"] as const).includes(candidate.method)) {
    throw new NavigationEligibilityError("method");
  }
  const target = checkedTarget(candidate);
  validateReflection(candidate, target);
  const transitionName = checkedTransitionName(candidate.transitionName);
  const enhanced = supportsDocumentEnhancement(candidate, target);

  return Object.freeze({
    target: new URL(target.href),
    method: candidate.method,
    history: candidate.history,
    prefetch: enhanced ? candidate.prefetch : "none",
    transitionName: enhanced && candidate.method === "GET" ? transitionName : null,
  });
}

export function isSameDocumentFragment(current: URL, target: URL): boolean {
  return sameDocumentFragment(current, target);
}
