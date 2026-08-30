import type { StimulusContinuity, StimulusContinuityRoot } from "./port.js";

const CONTROLLER_SELECTOR = "[data-controller]";
const ISLAND_SELECTOR = "[data-suprnova-live-island]";
const DOCUMENT_KEY_ATTRIBUTE = "data-suprnova-live-document-key";
const LIVE_KEY_ATTRIBUTE = "data-suprnova-live-key";
const SAFE_IDENTITY = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
const MAX_CONTROLLER_ROOTS_PER_SCOPE = 1_024;

export class StimulusLifecycleError extends Error {
  constructor(readonly code: "invalid_scope" | "invalid_identity" | "resource_exhausted") {
    super(`stimulus_lifecycle_${code}`);
  }
}

function elementLike(value: unknown): value is Element {
  if (typeof value !== "object" || value === null || Reflect.get(value, "nodeType") !== 1) {
    return false;
  }
  return ["closest", "getAttribute", "matches", "querySelectorAll"].every(
    (name) => typeof Reflect.get(value, name) === "function",
  );
}

function stableIdentity(element: Element): string | null {
  const candidate =
    element.getAttribute(LIVE_KEY_ATTRIBUTE) ?? element.getAttribute(DOCUMENT_KEY_ATTRIBUTE);
  if (candidate === null) return null;
  if (!SAFE_IDENTITY.test(candidate)) throw new StimulusLifecycleError("invalid_identity");
  return candidate;
}

function ownedBy(scopeOwner: Element | null, candidate: Element): boolean {
  const owner = candidate.closest(ISLAND_SELECTOR);
  return scopeOwner === null ? owner === null : owner === scopeOwner;
}

export function captureStimulusContinuity(scope: Element): StimulusContinuity {
  if (!elementLike(scope)) throw new StimulusLifecycleError("invalid_scope");
  const scopeOwner = scope.closest(ISLAND_SELECTOR);
  const candidates: Element[] = [];
  if (scope.matches(CONTROLLER_SELECTOR)) candidates.push(scope);
  candidates.push(...scope.querySelectorAll(CONTROLLER_SELECTOR));
  if (candidates.length > MAX_CONTROLLER_ROOTS_PER_SCOPE) {
    throw new StimulusLifecycleError("resource_exhausted");
  }

  const identities = new Set<string>();
  const roots: StimulusContinuityRoot[] = [];
  for (const element of candidates) {
    if (!ownedBy(scopeOwner, element)) continue;
    const identity = stableIdentity(element);
    if (identity === null) continue;
    if (identities.has(identity)) throw new StimulusLifecycleError("invalid_identity");
    identities.add(identity);
    roots.push(Object.freeze({ element, identity }));
  }
  return Object.freeze({
    roots: Object.freeze(roots),
    scope,
    scopeIdentity: stableIdentity(scope),
  });
}
