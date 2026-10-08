import type { MorphControl } from "./controls.js";
import type { MorphIdentityEntry, MorphPlan } from "./types.js";

import { stableKeyOf } from "./keys.js";
const STREAM_STATE_ATTRIBUTE = "data-live-stream-state";
const STREAM_STATUS_ATTRIBUTE = "data-live-stream-status";
const STREAM_ROOT_ATTRIBUTES: ReadonlySet<string> = new Set([
  "aria-busy",
  "data-live-stream-motion",
  STREAM_STATE_ATTRIBUTE,
]);
const STREAM_STATUS_ATTRIBUTES: ReadonlySet<string> = new Set(["aria-atomic", "aria-live", "role"]);

interface ControlAncestor {
  readonly control: MorphControl;
  readonly root: Element;
}

function asElement(node: Node): Element | null {
  return node.nodeType === 1 ? (node as Element) : node.parentElement;
}

function controlAncestor(plan: MorphPlan, node: Node): ControlAncestor | null {
  let element = asElement(node);
  while (element !== null) {
    const key = stableKeyOf(element);
    const control = key === null ? undefined : plan.controls.byKey.get(key);
    if (control !== undefined) return { control, root: element };
    if (element === plan.currentRoot || element === plan.replacementRoot) return null;
    element = element.parentElement;
  }
  return null;
}

// The runtime projects a stream state onto the island root and announces
// into the status elements the view renders; the server's re-render carries
// neither, so a morph that copied it would show the disconnected default
// over a current stream until the next state change.
// The stream the root declares, with its modifiers: a replacement that drops
// or changes the stream hands the status back to the server's baseline, which
// the async feature restores after the morph.
function streamDeclaration(root: Element): string | null {
  for (const { name, value } of root.attributes) {
    if (name === "live:stream" || name.startsWith("live:stream.")) return `${name}=${value}`;
  }
  return null;
}

function projectsStreamStatus(plan: MorphPlan): boolean {
  if (!plan.currentRoot.hasAttribute(STREAM_STATE_ATTRIBUTE)) return false;
  const declared = streamDeclaration(plan.currentRoot);
  return declared !== null && declared === streamDeclaration(plan.replacementRoot);
}

function streamStatusTarget(plan: MorphPlan, node: Node): Element | null {
  let element = asElement(node);
  while (element !== null) {
    if (element === plan.currentRoot || element === plan.replacementRoot) return null;
    if (element.hasAttribute(STREAM_STATUS_ATTRIBUTE)) return element;
    if (plan.identity.nestedCurrentRoots.has(element)) return null;
    element = element.parentElement;
  }
  return null;
}

function insideStreamStatus(plan: MorphPlan, node: Node): boolean {
  if (!projectsStreamStatus(plan)) return false;
  const target = streamStatusTarget(plan, node);
  return target !== null && target !== node;
}

export function preservesStreamStatus(plan: MorphPlan, name: string, node: Element): boolean {
  if (!projectsStreamStatus(plan)) return false;
  if (node === plan.currentRoot) return STREAM_ROOT_ATTRIBUTES.has(name);
  return streamStatusTarget(plan, node) === node && STREAM_STATUS_ATTRIBUTES.has(name);
}

export function preservesAttribute(plan: MorphPlan, node: Element): boolean {
  const controlled = controlAncestor(plan, node);
  if (controlled?.root !== node) return false;
  return (
    controlled.control.kind === "preserve" ||
    (controlled.control.kind === "ignore" && controlled.control.attributes === "browser")
  );
}

export function skipsNodeMorph(plan: MorphPlan, current: Node, replacement: Node): boolean {
  if (insideStreamStatus(plan, current) || insideStreamStatus(plan, replacement)) return true;
  const controlled = controlAncestor(plan, current) ?? controlAncestor(plan, replacement);
  if (controlled?.control.kind !== "ignore") return false;
  if (controlled.root !== current && controlled.root !== replacement) return true;
  return controlled.control.attributes === "browser";
}

export function skipsNodeAddition(plan: MorphPlan, node: Node): boolean {
  if (insideStreamStatus(plan, node)) return true;
  const controlled = controlAncestor(plan, node);
  return controlled?.control.kind === "ignore" && controlled.root !== node;
}

export function skipsNodeRemoval(plan: MorphPlan, node: Node): boolean {
  if (insideStreamStatus(plan, node)) return true;
  const controlled = controlAncestor(plan, node);
  return controlled?.control.kind === "ignore" && controlled.root !== node;
}

export function forcesReplacement(plan: MorphPlan, entry: MorphIdentityEntry): boolean {
  return entry.kind === "live_key" && plan.controls.byKey.get(entry.value)?.kind === "replace";
}
