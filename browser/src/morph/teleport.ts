import type { MorphPlan } from "./types.js";

const SAFE_TARGET = /^#[A-Za-z][A-Za-z0-9_-]{0,127}$/u;
const ISLAND_ATTRIBUTE = "data-suprnova-live-island";
const KEY_ATTRIBUTE = "data-suprnova-live-key";
const CONTROL_ATTRIBUTE = "live:teleport";
const SAFE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export interface ActiveTeleport {
  readonly key: string;
  readonly node: Element;
  readonly target: string;
}

export interface TeleportTargetPort {
  resolve(target: string, ownerRoot: Element): Element | null;
  active?(ownerRoot: Element): readonly ActiveTeleport[];
}

interface TeleportRecord extends ActiveTeleport {
  readonly ownerRoot: Element;
  placeholder: Comment;
  targetElement: Element;
}

interface TransitionEntry {
  readonly record: TeleportRecord;
  readonly originParent: Node;
  readonly originNext: Node | null;
}

export interface TeleportTransition {
  readonly ownerRoot: Element;
  readonly plan: MorphPlan;
  readonly entries: readonly TransitionEntry[];
  readonly focus: Element | null;
  active: boolean;
}

function nearestIsland(element: Element): Element | null {
  let current: Element | null = element;
  while (current !== null) {
    if (current.hasAttribute(ISLAND_ATTRIBUTE)) return current;
    current = current.parentElement;
  }
  return null;
}

function containedBy(candidate: Element | null, ancestor: Element): boolean {
  let current = candidate;
  while (current !== null) {
    if (current === ancestor) return true;
    current = current.parentElement;
  }
  return false;
}

function walk(root: Element): readonly Element[] {
  const elements: Element[] = [];
  const stack = [...root.children];
  while (stack.length > 0) {
    const element = stack.pop();
    if (element === undefined) break;
    if (element.hasAttribute(ISLAND_ATTRIBUTE)) continue;
    elements.push(element);
    stack.push(...element.children);
  }
  return elements;
}

function containsNestedIsland(root: Element): boolean {
  const stack = [...root.children];
  while (stack.length > 0) {
    const element = stack.pop();
    if (element === undefined) break;
    if (element.hasAttribute(ISLAND_ATTRIBUTE)) return true;
    stack.push(...element.children);
  }
  return false;
}

function teleportCandidate(element: Element): ActiveTeleport | null {
  const target = element.getAttribute(CONTROL_ATTRIBUTE);
  if (target === null) return null;
  const controls = [...element.attributes].filter(({ name }) =>
    ["live:preserve", "live:ignore", "live:replace", "live:persist", "live:teleport"].some(
      (control) => name === control || name.startsWith(`${control}.`),
    ),
  );
  const key = element.getAttribute(KEY_ATTRIBUTE);
  if (controls.length !== 1 || !SAFE_TARGET.test(target) || key === null || !SAFE_KEY.test(key)) {
    throw new Error("teleport_control_invalid");
  }
  if (containsNestedIsland(element)) {
    throw new Error("teleport_nested_island");
  }
  return Object.freeze({ key, node: element, target });
}

function findKey(root: Element, key: string): Element | null {
  return walk(root).find((element) => element.getAttribute(KEY_ATTRIBUTE) === key) ?? null;
}

export class TeleportRegistry implements TeleportTargetPort {
  readonly #document: Document;
  readonly #targets = new Map<string, Element>();
  readonly #records = new Map<Element, Map<string, TeleportRecord>>();
  readonly #controlledMoves = new WeakMap<Node, number>();

  constructor(document: Document) {
    this.#document = document;
    const candidates = new Map<string, Element[]>();
    for (const element of document.querySelectorAll("[id]")) {
      const id = element.getAttribute("id");
      if (id === null || !SAFE_TARGET.test(`#${id}`)) continue;
      const entries = candidates.get(id) ?? [];
      entries.push(element);
      candidates.set(id, entries);
    }
    for (const [id, entries] of candidates) {
      if (entries.length === 1) this.#targets.set(id, entries[0] ?? fail());
    }
  }

  resolve(target: string, ownerRoot: Element): Element | null {
    if (!SAFE_TARGET.test(target) || ownerRoot.ownerDocument !== this.#document) return null;
    const id = target.slice(1);
    const authorized = this.#targets.get(id);
    if (authorized?.isConnected !== true) return null;
    let current: readonly Element[];
    try {
      current = [...this.#document.querySelectorAll(target)];
    } catch {
      return null;
    }
    if (current.length !== 1 || current[0] !== authorized) return null;
    const owner = nearestIsland(authorized);
    return owner === null || owner === ownerRoot ? authorized : null;
  }

  active(ownerRoot: Element): readonly ActiveTeleport[] {
    return Object.freeze(
      [...(this.#records.get(ownerRoot)?.values() ?? [])].map(({ key, node, target }) =>
        Object.freeze({ key, node, target }),
      ),
    );
  }

  mount(ownerRoot: Element): void {
    if (ownerRoot.ownerDocument !== this.#document) throw new Error("teleport_owner_invalid");
    const candidates = walk(ownerRoot)
      .map(teleportCandidate)
      .filter((candidate): candidate is ActiveTeleport => candidate !== null);
    const keys = new Set<string>();
    const prepared = candidates.map((candidate) => {
      if (keys.has(candidate.key)) throw new Error("teleport_key_duplicate");
      keys.add(candidate.key);
      const target = this.resolve(candidate.target, ownerRoot);
      if (target === null || target === candidate.node || containedBy(target, candidate.node)) {
        throw new Error("teleport_target_invalid");
      }
      return { candidate, target };
    });
    const records = this.#records.get(ownerRoot) ?? new Map<string, TeleportRecord>();
    if (records.size > 0 || prepared.some(({ candidate }) => records.has(candidate.key))) {
      throw new Error("teleport_mount_repeated");
    }
    this.#records.set(ownerRoot, records);
    for (const { candidate, target } of prepared) {
      const placeholder = this.#document.createComment(`suprnova-live-teleport:${candidate.key}`);
      const record: TeleportRecord = {
        ...candidate,
        ownerRoot,
        placeholder,
        targetElement: target,
      };
      records.set(candidate.key, record);
      this.#moveOut(record);
    }
  }

  begin(plan: MorphPlan): TeleportTransition {
    const records = this.#records.get(plan.currentRoot);
    const entries: TransitionEntry[] = [];
    const active = this.#document.activeElement;
    const activeElement = active?.nodeType === 1 ? active : null;
    let focus: Element | null = null;
    for (const record of records?.values() ?? []) {
      if (activeElement !== null && containedBy(activeElement, record.node)) focus = activeElement;
      const originParent = record.placeholder.parentNode ?? record.placeholder.parentElement;
      if (originParent === null) throw new Error("teleport_origin_missing");
      const originNext = record.placeholder.nextSibling;
      record.placeholder.replaceWith(record.node);
      this.#markControlledMove(record.node);
      entries.push({ originNext, originParent, record });
    }
    return {
      active: true,
      entries: Object.freeze(entries),
      focus,
      ownerRoot: plan.currentRoot,
      plan,
    };
  }

  commit(transition: TeleportTransition, ownerRoot: Element): void {
    this.#assertTransition(transition, ownerRoot);
    const records = this.#records.get(ownerRoot) ?? new Map<string, TeleportRecord>();
    const retained = new Set<string>();
    const prepared: Readonly<{ node: Element; record: TeleportRecord }>[] = [];
    for (const binding of transition.plan.controls.bindings) {
      if (binding.control.kind !== "teleport" || binding.replacement === null) continue;
      const node = findKey(ownerRoot, binding.control.key);
      const target = transition.plan.controls.teleportTargets.get(binding.control.key);
      if (node === null || target === undefined) throw new Error("teleport_commit_invalid");
      const existing = records.get(binding.control.key);
      if (existing !== undefined && existing.node !== node) {
        throw new Error("teleport_commit_invalid");
      }
      const record: TeleportRecord =
        existing ??
        ({
          key: binding.control.key,
          node,
          ownerRoot,
          placeholder: this.#document.createComment(
            `suprnova-live-teleport:${binding.control.key}`,
          ),
          target: binding.control.target,
          targetElement: target,
        } satisfies TeleportRecord);
      record.targetElement = target;
      retained.add(record.key);
      prepared.push({ node, record });
    }
    for (const { record } of prepared) {
      records.set(record.key, record);
      this.#moveOut(record);
    }
    for (const [key, record] of records) {
      if (retained.has(key)) continue;
      record.node.remove();
      record.placeholder.remove();
      records.delete(key);
    }
    if (records.size === 0) this.#records.delete(ownerRoot);
    else this.#records.set(ownerRoot, records);
    this.#restoreFocus(transition.focus);
    transition.active = false;
  }

  rollback(transition: TeleportTransition): void {
    this.#assertTransition(transition, transition.ownerRoot);
    for (const { originNext, originParent, record } of transition.entries) {
      record.placeholder.remove();
      const before = originNext?.parentNode === originParent ? originNext : null;
      originParent.insertBefore(record.placeholder, before);
      if (record.node.parentNode !== record.targetElement) {
        record.targetElement.append(record.node);
        this.#markControlledMove(record.node);
      }
    }
    this.#restoreFocus(transition.focus);
    transition.active = false;
  }

  disposeOwner(ownerRoot: Element): void {
    const records = this.#records.get(ownerRoot);
    if (records === undefined) return;
    for (const record of records.values()) {
      record.node.remove();
      record.placeholder.remove();
    }
    records.clear();
    this.#records.delete(ownerRoot);
  }

  consumeControlledMove(node: Node): boolean {
    const remaining = this.#controlledMoves.get(node) ?? 0;
    if (remaining === 0) return false;
    if (remaining === 1) this.#controlledMoves.delete(node);
    else this.#controlledMoves.set(node, remaining - 1);
    return true;
  }

  #moveOut(record: TeleportRecord): void {
    const active = this.#document.activeElement;
    const activeElement = active?.nodeType === 1 ? active : null;
    const restoreFocus = activeElement !== null && containedBy(activeElement, record.node);
    record.node.replaceWith(record.placeholder);
    record.targetElement.append(record.node);
    this.#markControlledMove(record.node);
    if (restoreFocus) this.#restoreFocus(activeElement);
  }

  #markControlledMove(node: Node): void {
    this.#controlledMoves.set(node, (this.#controlledMoves.get(node) ?? 0) + 2);
  }

  #restoreFocus(element: Element | null): void {
    if (element?.isConnected !== true) return;
    const focusable = element as Element & { focus?: (options?: FocusOptions) => void };
    focusable.focus?.({ preventScroll: true });
  }

  #assertTransition(transition: TeleportTransition, ownerRoot: Element): void {
    if (!transition.active || transition.ownerRoot !== ownerRoot) {
      throw new Error("teleport_transition_invalid");
    }
  }
}

function fail(): never {
  throw new Error("teleport_target_invalid");
}
