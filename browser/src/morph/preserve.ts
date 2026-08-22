import type { MorphControl } from "./controls.js";
import type { MorphIdentityEntry, MorphPlan } from "./types.js";

const KEY_ATTRIBUTE = "data-suprnova-live-key";

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
    const key = element.getAttribute(KEY_ATTRIBUTE);
    const control = key === null ? undefined : plan.controls.byKey.get(key);
    if (control !== undefined) return { control, root: element };
    if (element === plan.currentRoot || element === plan.replacementRoot) return null;
    element = element.parentElement;
  }
  return null;
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
  const controlled = controlAncestor(plan, current) ?? controlAncestor(plan, replacement);
  if (controlled?.control.kind !== "ignore") return false;
  if (controlled.root !== current && controlled.root !== replacement) return true;
  return controlled.control.attributes === "browser";
}

export function skipsNodeAddition(plan: MorphPlan, node: Node): boolean {
  const controlled = controlAncestor(plan, node);
  return controlled?.control.kind === "ignore" && controlled.root !== node;
}

export function skipsNodeRemoval(plan: MorphPlan, node: Node): boolean {
  const controlled = controlAncestor(plan, node);
  return controlled?.control.kind === "ignore" && controlled.root !== node;
}

export function forcesReplacement(plan: MorphPlan, entry: MorphIdentityEntry): boolean {
  return entry.kind === "live_key" && plan.controls.byKey.get(entry.value)?.kind === "replace";
}
