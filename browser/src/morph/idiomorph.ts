import { Idiomorph } from "idiomorph";

import { isValidatedMorphPlan } from "./preflight.js";
import type {
  MorphAdapter,
  MorphClock,
  MorphHooks,
  MorphIdentityEntry,
  MorphPlan,
  MorphResult,
} from "./types.js";

const morphProvenance = new WeakSet<Node>();
const PROVENANCE_ATTRIBUTE = "data-suprnova-live-internal-provenance";
const IDENTITY_ATTRIBUTE = "data-suprnova-live-internal-identity";
const DESIRED_ID_ATTRIBUTE = "data-suprnova-live-internal-desired-id";
let surrogateSequence = 0;

function defaultClock(): number {
  return performance.now();
}

function connected(element: Element): boolean {
  return element.isConnected;
}

function setId(element: Element, value: string | null): void {
  if (value === null) element.removeAttribute("id");
  else element.setAttribute("id", value);
}

function pairMap(entries: readonly MorphIdentityEntry[]): {
  readonly current: ReadonlyMap<Node, MorphIdentityEntry>;
  readonly replacement: ReadonlyMap<string, MorphIdentityEntry>;
} {
  const current = new Map<Node, MorphIdentityEntry>();
  const replacement = new Map<string, MorphIdentityEntry>();
  for (const entry of entries) {
    if (entry.current !== null) current.set(entry.current, entry);
    if (entry.replacement !== null) replacement.set(entry.token, entry);
  }
  return { current, replacement };
}

function elementsWithin(node: Node): Element[] {
  const elements: Element[] = [];
  const stack: Node[] = [node];
  while (stack.length > 0) {
    const candidate = stack.pop();
    if (candidate === undefined) break;
    if (candidate.nodeType === 1) elements.push(candidate as Element);
    stack.push(...candidate.childNodes);
  }
  return elements;
}

function replacementIdentity(
  node: Node,
  pairs: ReadonlyMap<string, MorphIdentityEntry>,
): MorphIdentityEntry | undefined {
  if (node.nodeType !== 1) return undefined;
  const token = (node as Element).getAttribute(IDENTITY_ATTRIBUTE);
  return token === null ? undefined : pairs.get(token);
}

function approvedNode(node: Node, markers: ReadonlySet<string>): boolean {
  let element = node.nodeType === 1 ? (node as Element) : (node.parentElement ?? null);
  while (element !== null) {
    if (typeof element.getAttribute !== "function") return false;
    const marker = element.getAttribute(PROVENANCE_ATTRIBUTE);
    if (marker !== null) return markers.has(marker);
    element = element.parentElement;
  }
  return false;
}

function markApprovedReplacement(plan: MorphPlan, prefix: string): ReadonlySet<string> {
  const markers = new Set<string>();
  const elements = elementsWithin(plan.replacementRoot);
  for (const [index, element] of elements.entries()) {
    const marker = `${prefix}p${index.toString(36)}`;
    markers.add(marker);
    element.setAttribute(PROVENANCE_ATTRIBUTE, marker);
  }
  for (const entry of plan.identity.entries) {
    if (entry.replacement !== null) {
      entry.replacement.setAttribute(IDENTITY_ATTRIBUTE, entry.token);
    }
  }
  return markers;
}

function restoreInternalTree(root: Element): void {
  for (const element of elementsWithin(root)) {
    if (element.hasAttribute(DESIRED_ID_ATTRIBUTE)) {
      const desired = element.getAttribute(DESIRED_ID_ATTRIBUTE);
      setId(element, desired === "" ? null : desired);
    }
    element.removeAttribute(PROVENANCE_ATTRIBUTE);
    element.removeAttribute(IDENTITY_ATTRIBUTE);
    element.removeAttribute(DESIRED_ID_ATTRIBUTE);
  }
}

function recordProvenance(node: Node): void {
  morphProvenance.add(node);
  for (const child of node.childNodes) recordProvenance(child);
}

function result(plan: MorphPlan): MorphResult {
  return Object.freeze({
    inserted: plan.identity.inserted,
    moved: plan.identity.moved,
    removed: plan.identity.removed,
    root: plan.currentRoot,
  });
}

export class IdiomorphAdapter implements MorphAdapter {
  readonly #clock: MorphClock;

  constructor(clock: MorphClock = defaultClock) {
    this.#clock = clock;
  }

  apply(plan: MorphPlan, hooks: MorphHooks): MorphResult {
    if (!isValidatedMorphPlan(plan)) throw new Error("morph_plan_invalid");
    if (
      !connected(plan.currentRoot) ||
      plan.replacementRoot.ownerDocument === plan.currentRoot.ownerDocument
    ) {
      throw new Error("morph_plan_stale");
    }
    const startedAt = this.#clock();
    let calls = 0;
    const checkBudget = (): void => {
      calls += 1;
      if (calls > plan.limits.maxHookCalls) throw new Error("morph_hook_limit");
      if (this.#clock() - startedAt > plan.limits.deadlineMs) {
        throw new Error("morph_deadline_exceeded");
      }
    };
    const pairs = pairMap(plan.identity.entries);
    const savedIds = new Map<Element, string | null>();
    const desiredIds = new Map<Element, string | null>();
    const prefix = `__suprnova_live_${surrogateSequence.toString(36)}_`;
    surrogateSequence += 1;
    const provenanceMarkers = markApprovedReplacement(plan, prefix);
    for (const entry of plan.identity.entries) {
      if (entry.kind === "id") continue;
      const current = entry.current;
      const replacement = entry.replacement;
      if (current !== null) {
        savedIds.set(current, current.getAttribute("id"));
        desiredIds.set(current, replacement?.getAttribute("id") ?? current.getAttribute("id"));
      }
      if (replacement !== null) {
        savedIds.set(replacement, replacement.getAttribute("id"));
        desiredIds.set(replacement, replacement.getAttribute("id"));
        replacement.setAttribute(DESIRED_ID_ATTRIBUTE, replacement.getAttribute("id") ?? "");
      }
      const surrogate = `${prefix}${savedIds.size.toString(36)}`;
      if (current !== null) setId(current, surrogate);
      if (replacement !== null) setId(replacement, surrogate);
    }
    try {
      checkBudget();
      hooks.beforeMorph?.(plan);
      Idiomorph.morph(plan.currentRoot, plan.replacementRoot, {
        callbacks: {
          beforeAttributeUpdated: (name, node) => {
            checkBudget();
            if (node === plan.currentRoot && name === "data-suprnova-live-status") return false;
            return undefined;
          },
          afterNodeAdded: (node) => {
            checkBudget();
            if (!approvedNode(node, provenanceMarkers)) {
              throw new Error("morph_unapproved_node");
            }
            recordProvenance(node);
            hooks.afterNodeAdded?.(node);
            if (node.nodeType === 1) restoreInternalTree(node as Element);
          },
          afterNodeMorphed: (current, replacement) => {
            checkBudget();
            hooks.afterNodeMorphed?.(current, replacement);
          },
          afterNodeRemoved: (node) => {
            checkBudget();
            hooks.afterNodeRemoved?.(node);
          },
          beforeNodeAdded: (node) => {
            checkBudget();
            if (!approvedNode(node, provenanceMarkers)) {
              throw new Error("morph_unapproved_node");
            }
            const entry = replacementIdentity(node, pairs.replacement);
            if (entry?.current !== null && entry?.current !== undefined) {
              throw new Error("morph_identity_recreated");
            }
            hooks.beforeNodeAdded?.(node);
          },
          beforeNodeMorphed: (current, replacement) => {
            checkBudget();
            const oldEntry = pairs.current.get(current);
            const newEntry = replacementIdentity(replacement, pairs.replacement);
            if (
              (oldEntry !== undefined || newEntry !== undefined) &&
              oldEntry?.token !== newEntry?.token
            ) {
              throw new Error("morph_identity_mismatch");
            }
            hooks.beforeNodeMorphed?.(current, replacement);
            if (plan.identity.nestedCurrentRoots.has(current as Element)) return false;
            return undefined;
          },
          beforeNodeRemoved: (node) => {
            checkBudget();
            const entry = pairs.current.get(node);
            if (entry !== undefined && entry.replacement !== null) {
              throw new Error("morph_identity_removed");
            }
            hooks.beforeNodeRemoved?.(node);
          },
        },
        morphStyle: "outerHTML",
        restoreFocus: false,
      });
      if (!connected(plan.currentRoot)) throw new Error("morph_root_replaced");
      const applied = result(plan);
      checkBudget();
      hooks.afterMorph?.(applied);
      return applied;
    } finally {
      for (const [element, value] of savedIds) setId(element, value);
      for (const [element, value] of desiredIds) {
        if (element.isConnected) setId(element, value);
      }
      restoreInternalTree(plan.currentRoot);
      restoreInternalTree(plan.replacementRoot);
    }
  }
}

export function hasMorphProvenance(node: Node): boolean {
  return morphProvenance.has(node);
}
