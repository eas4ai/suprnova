import { parseMorphHtml, type MorphHtmlParser } from "./html.js";
import { planMorphIdentity } from "./keys.js";
import { validateMorphLimits } from "./limits.js";
import type { MorphAuthority, MorphLimits, MorphPlan } from "./types.js";

const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
const validatedPlans = new WeakSet();

export class MorphPreflightError extends Error {
  constructor(readonly detail: string) {
    super("morph_preflight_invalid");
    this.name = "MorphPreflightError";
  }
}

export interface MorphPreflightInput {
  readonly currentRoot: HTMLElement;
  readonly html: string;
  readonly authority: MorphAuthority;
  readonly limits: MorphLimits;
  readonly parser?: MorphHtmlParser;
}

function fail(detail: string): never {
  throw new MorphPreflightError(detail);
}

function attribute(element: Element, name: string, expected: string): void {
  if (element.getAttribute(name) !== expected) fail(name);
}

function validateCurrent(root: HTMLElement, authority: MorphAuthority): void {
  if (root.namespaceURI !== HTML_NAMESPACE || !root.isConnected) {
    fail("current_root");
  }
  attribute(root, "data-suprnova-live-island", "");
  attribute(root, "data-suprnova-live-component", authority.component);
  attribute(root, "data-suprnova-live-slot", authority.slot);
  attribute(root, "data-suprnova-live-root", authority.slot);
  attribute(root, "data-suprnova-live-document-key", authority.documentKey);
  attribute(root, "data-suprnova-live-snapshot-kind", "instance");
  attribute(root, "data-suprnova-live-instance", authority.instanceId);
}

function validateReplacement(
  current: HTMLElement,
  replacement: HTMLElement,
  authority: MorphAuthority,
): void {
  if (
    replacement.localName !== current.localName ||
    replacement.namespaceURI !== current.namespaceURI ||
    replacement.ownerDocument === current.ownerDocument
  ) {
    fail("replacement_root");
  }
  attribute(replacement, "data-suprnova-live-island", "");
  attribute(replacement, "data-suprnova-live-component", authority.component);
  attribute(replacement, "data-suprnova-live-slot", authority.slot);
  attribute(replacement, "data-suprnova-live-root", authority.slot);
  attribute(replacement, "data-suprnova-live-document-key", authority.documentKey);
  attribute(replacement, "data-suprnova-live-snapshot-kind", "instance");
  attribute(replacement, "data-suprnova-live-instance", authority.instanceId);
  attribute(replacement, "data-suprnova-live-revision", authority.successorRevision.toString(10));
  attribute(replacement, "data-suprnova-live-snapshot", authority.encodedSnapshot);
  for (const name of [
    "data-suprnova-live-contract",
    "data-suprnova-live-protocol-min",
    "data-suprnova-live-lazy-complete",
  ]) {
    if (replacement.getAttribute(name) !== current.getAttribute(name)) fail(name);
  }
}

export function preflightIslandMorph(input: MorphPreflightInput): MorphPlan {
  try {
    validateMorphLimits(input.limits);
    validateCurrent(input.currentRoot, input.authority);
    const replacement = parseMorphHtml(
      input.currentRoot.ownerDocument,
      input.html,
      input.limits,
      input.parser,
    );
    validateReplacement(input.currentRoot, replacement, input.authority);
    const plan = Object.freeze({
      currentRoot: input.currentRoot,
      identity: planMorphIdentity(input.currentRoot, replacement, input.limits),
      limits: input.limits,
      replacementRoot: replacement,
    });
    validatedPlans.add(plan);
    return plan;
  } catch (error: unknown) {
    if (error instanceof MorphPreflightError) throw error;
    return fail(error instanceof Error ? error.message : "unknown");
  }
}

export function isValidatedMorphPlan(plan: MorphPlan): boolean {
  return validatedPlans.has(plan);
}
