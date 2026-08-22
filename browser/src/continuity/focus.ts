import type { MorphIdentityEntry, MorphPlan } from "../morph/types.js";
import {
  ContinuityError,
  type ContinuityBudget,
  type ContinuityLimits,
  type FocusContinuity,
  type SelectionRecord,
} from "./types.js";

const MAX_SELECTION_PATH_DEPTH = 32;
const FOCUS_FALLBACK_SELECTOR = "[data-suprnova-live-focus-fallback]";

function containedBy(root: Element, candidate: Node | null): boolean {
  return candidate !== null && (candidate === root || root.contains(candidate));
}

function deepestIdentity(plan: MorphPlan, candidate: Node | null): MorphIdentityEntry | null {
  let best: MorphIdentityEntry | null = null;
  for (const entry of plan.identity.entries) {
    if (entry.current === null || !containedBy(entry.current, candidate)) continue;
    const bestElement = best?.current ?? null;
    if (bestElement === null || containedBy(bestElement, entry.current)) best = entry;
  }
  return best;
}

function focusVisible(element: Element): boolean {
  try {
    return element.matches(":focus-visible");
  } catch {
    return false;
  }
}

export function captureFocus(plan: MorphPlan): FocusContinuity {
  const active = plan.currentRoot.ownerDocument.activeElement;
  const entry = deepestIdentity(plan, active);
  const element =
    active instanceof HTMLElement && (entry !== null || containedBy(plan.currentRoot, active))
      ? active
      : null;
  return Object.freeze({
    element,
    focusedKey: entry?.token ?? null,
    focusVisible: element === null ? false : focusVisible(element),
  });
}

function textControl(element: Element): element is HTMLInputElement | HTMLTextAreaElement {
  if (element.tagName === "TEXTAREA") return true;
  if (element.tagName !== "INPUT") return false;
  const type = (element as HTMLInputElement).type.toLowerCase();
  return ["email", "password", "search", "tel", "text", "url"].includes(type);
}

function nodePath(root: Node, node: Node): readonly number[] | null {
  const path: number[] = [];
  let current: Node | null = node;
  while (current !== root) {
    const parent: Node | null = current.parentNode;
    if (parent === null || path.length >= MAX_SELECTION_PATH_DEPTH) return null;
    const index = [...parent.childNodes].indexOf(current as ChildNode);
    if (index < 0) return null;
    path.unshift(index);
    current = parent;
  }
  return Object.freeze(path);
}

function resolvePath(root: Node, path: readonly number[]): Node | null {
  let current = root;
  for (const index of path) {
    const next = current.childNodes[index];
    if (next === undefined) return null;
    current = next;
  }
  return current;
}

function contenteditableSelection(plan: MorphPlan): SelectionRecord | null {
  const selection = plan.currentRoot.ownerDocument.getSelection();
  if (selection?.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0);
  const entry = deepestIdentity(plan, range.commonAncestorContainer);
  if (!(entry?.current instanceof HTMLElement) || !entry.current.isContentEditable) return null;
  const startPath = nodePath(entry.current, range.startContainer);
  const endPath = nodePath(entry.current, range.endContainer);
  if (startPath === null || endPath === null) return null;
  return Object.freeze({
    endOffset: range.endOffset,
    endPath,
    identity: entry.token,
    kind: "contenteditable",
    root: entry.current,
    startOffset: range.startOffset,
    startPath,
  });
}

export function captureSelections(
  plan: MorphPlan,
  limits: ContinuityLimits,
  budget: ContinuityBudget,
): readonly SelectionRecord[] {
  void budget;
  const records: SelectionRecord[] = [];
  for (const entry of plan.identity.entries) {
    const element = entry.current;
    if (element === null || !textControl(element)) continue;
    const start = element.selectionStart;
    const end = element.selectionEnd;
    if (start === null || end === null) continue;
    records.push(
      Object.freeze({
        direction: element.selectionDirection ?? "none",
        element,
        end,
        identity: entry.token,
        kind: "control",
        start,
      }),
    );
  }
  const contenteditable = contenteditableSelection(plan);
  if (contenteditable !== null) records.push(contenteditable);
  if (records.length > limits.maxSelections) throw new ContinuityError("resource_exhausted");
  return Object.freeze(records);
}

function boundedOffset(node: Node, offset: number): number {
  const maximum =
    node.nodeType === Node.TEXT_NODE ? (node.nodeValue?.length ?? 0) : node.childNodes.length;
  return Math.min(Math.max(offset, 0), maximum);
}

export function restoreSelections(root: HTMLElement, selections: readonly SelectionRecord[]): void {
  for (const selection of selections) {
    if (selection.kind === "control") {
      if (!selection.element.isConnected || !root.contains(selection.element)) continue;
      const maximum = selection.element.value.length;
      selection.element.setSelectionRange(
        Math.min(selection.start, maximum),
        Math.min(selection.end, maximum),
        selection.direction,
      );
      continue;
    }
    if (!selection.root.isConnected || !root.contains(selection.root)) continue;
    const start = resolvePath(selection.root, selection.startPath);
    const end = resolvePath(selection.root, selection.endPath);
    const documentSelection = root.ownerDocument.getSelection();
    if (start === null || end === null || documentSelection === null) continue;
    const range = root.ownerDocument.createRange();
    range.setStart(start, boundedOffset(start, selection.startOffset));
    range.setEnd(end, boundedOffset(end, selection.endOffset));
    documentSelection.removeAllRanges();
    documentSelection.addRange(range);
  }
}

function focusable(element: HTMLElement): boolean {
  return (
    element.isConnected &&
    !element.hidden &&
    element.getAttribute("aria-hidden") !== "true" &&
    element.getAttribute("aria-disabled") !== "true" &&
    !element.hasAttribute("disabled") &&
    element.closest("[inert]") === null
  );
}

export function restoreFocus(root: HTMLElement, focus: FocusContinuity): void {
  if (focus.element !== null && focusable(focus.element)) {
    if (root.contains(focus.element)) {
      focus.element.focus({ preventScroll: true });
      return;
    }
    if (focus.element.ownerDocument.activeElement === focus.element) return;
  }
  if (focus.focusedKey === null) return;
  const candidates = root.querySelectorAll<HTMLElement>(FOCUS_FALLBACK_SELECTOR);
  if (candidates.length > 1) throw new ContinuityError("invalid_identity");
  const declared = candidates[0];
  if (declared !== undefined && focusable(declared)) {
    declared.focus({ preventScroll: true });
    return;
  }
  if (!focusable(root)) return;
  if (!root.hasAttribute("tabindex")) root.setAttribute("tabindex", "-1");
  root.focus({ preventScroll: true });
}
