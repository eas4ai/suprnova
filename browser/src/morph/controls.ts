import { parseDirective } from "../directives/parser.js";
import type { IdentityPlan, MorphLimits } from "./types.js";
import type { TeleportTargetPort } from "./teleport.js";

const CONTROL_NAMES = new Set(["preserve", "ignore", "replace", "persist", "teleport"]);
const ISLAND_ATTRIBUTE = "data-suprnova-live-island";
const KEY_ATTRIBUTE = "data-suprnova-live-key";
const ENGINE_ATTRIBUTE_PREFIX = "data-suprnova-live-";
const SAFE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

export type MorphControl =
  | Readonly<{ kind: "preserve"; key: string }>
  | Readonly<{ kind: "ignore"; key: string; attributes: "server" | "browser" }>
  | Readonly<{ kind: "replace"; key: string }>
  | Readonly<{ kind: "persist"; key: string; destination: string }>
  | Readonly<{ kind: "teleport"; key: string; target: string }>;

export interface MorphControlBinding {
  readonly control: MorphControl;
  readonly current: Element | null;
  readonly replacement: Element | null;
}

export interface MorphControlPlan {
  readonly bindings: readonly MorphControlBinding[];
  readonly byKey: ReadonlyMap<string, MorphControl>;
  readonly byCurrent: ReadonlyMap<Element, MorphControl>;
  readonly byReplacement: ReadonlyMap<Element, MorphControl>;
  readonly teleportTargets: ReadonlyMap<string, Element>;
}

interface ControlCandidate {
  readonly control: MorphControl;
  readonly element: Element;
}

function fail(detail: string): never {
  throw new Error(`morph_control_${detail}`);
}

function directiveName(name: string): string | null {
  if (!name.startsWith("live:")) return null;
  return name.slice(5).split(".", 1)[0] ?? null;
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function stableKey(element: Element, limits: MorphLimits): string {
  const key = element.getAttribute(KEY_ATTRIBUTE);
  if (key === null || !SAFE_KEY.test(key) || utf8Length(key) > limits.maxKeyBytes) {
    return fail("key_invalid");
  }
  return key;
}

function descendantAuthority(element: Element): boolean {
  const stack = [...element.children];
  while (stack.length > 0) {
    const candidate = stack.pop();
    if (candidate === undefined) break;
    for (const attribute of candidate.attributes) {
      if (
        attribute.name.startsWith("live:") ||
        attribute.name.startsWith(ENGINE_ATTRIBUTE_PREFIX)
      ) {
        return true;
      }
    }
    stack.push(...candidate.children);
  }
  return false;
}

function controlFor(element: Element, limits: MorphLimits): MorphControl | null {
  const attributes = [...element.attributes];
  const controls = attributes.filter((attribute) => {
    const name = directiveName(attribute.name);
    return name !== null && CONTROL_NAMES.has(name);
  });
  if (controls.length === 0) return null;
  if (controls.length !== 1 || element.hasAttribute(ISLAND_ATTRIBUTE)) {
    return fail("combination_invalid");
  }
  const attribute = controls[0] ?? fail("combination_invalid");
  const names = attributes.map(({ name }) => name);
  const parsed = parseDirective(attribute.name, attribute.value, names);
  if (!parsed.ok) return fail(parsed.code);
  const key = stableKey(element, limits);
  switch (parsed.name) {
    case "preserve":
      if (parsed.modifiers.length !== 1 || parsed.modifiers[0] !== "self") {
        return fail("mode_invalid");
      }
      return Object.freeze({ key, kind: "preserve" });
    case "ignore": {
      const modifier = parsed.modifiers.length === 1 ? parsed.modifiers[0] : undefined;
      if (modifier !== "children" && modifier !== "subtree") return fail("mode_invalid");
      if (descendantAuthority(element)) return fail("ignored_authority");
      return Object.freeze({
        attributes: modifier === "children" ? "server" : "browser",
        key,
        kind: "ignore",
      });
    }
    case "replace":
      if (parsed.modifiers.length !== 1 || parsed.modifiers[0] !== "subtree") {
        return fail("mode_invalid");
      }
      return Object.freeze({ key, kind: "replace" });
    case "persist":
      if (parsed.modifiers.length !== 0) return fail("mode_invalid");
      return Object.freeze({ destination: parsed.value, key, kind: "persist" });
    case "teleport":
      if (parsed.modifiers.length !== 0 || !/^#[A-Za-z][A-Za-z0-9_-]{0,127}$/u.test(parsed.value)) {
        return fail("target_invalid");
      }
      return Object.freeze({ key, kind: "teleport", target: parsed.value });
    default:
      return fail("kind_invalid");
  }
}

function sameControl(current: MorphControl, replacement: MorphControl): boolean {
  if (current.kind !== replacement.kind || current.key !== replacement.key) return false;
  if (current.kind === "ignore" && replacement.kind === "ignore") {
    return current.attributes === replacement.attributes;
  }
  if (current.kind === "persist" && replacement.kind === "persist") {
    return current.destination === replacement.destination;
  }
  if (current.kind === "teleport" && replacement.kind === "teleport") {
    return current.target === replacement.target;
  }
  return true;
}

function scanControls(root: Element, limits: MorphLimits): Map<string, ControlCandidate> {
  const controls = new Map<string, ControlCandidate>();
  const stack: Readonly<{ element: Element; mobileAncestor: boolean }>[] = [...root.children]
    .reverse()
    .map((element) => ({ element, mobileAncestor: false }));
  while (stack.length > 0) {
    const entry = stack.pop();
    if (entry === undefined) break;
    const { element, mobileAncestor } = entry;
    if (element.hasAttribute(ISLAND_ATTRIBUTE)) continue;
    const control = controlFor(element, limits);
    if (
      control !== null &&
      mobileAncestor &&
      (control.kind === "persist" || control.kind === "teleport")
    ) {
      fail("nested_move");
    }
    if (control !== null) {
      if (controls.has(control.key)) fail("duplicate_key");
      controls.set(control.key, Object.freeze({ control, element }));
    }
    const nextMobile =
      mobileAncestor || control?.kind === "persist" || control?.kind === "teleport";
    const children = [...element.children];
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const child = children[index];
      if (child !== undefined) stack.push({ element: child, mobileAncestor: nextMobile });
    }
  }
  return controls;
}

function containedBy(candidate: Element, ancestor: Element): boolean {
  let current: Element | null = candidate;
  while (current !== null) {
    if (current === ancestor) return true;
    current = current.parentElement;
  }
  return false;
}

function externalIdentity(element: Element, limits: MorphLimits): string | null {
  const id = element.getAttribute("id");
  if (id !== null && (!SAFE_KEY.test(id) || utf8Length(id) > limits.maxKeyBytes)) {
    fail("active_teleport_identity");
  }
  const liveKey = element.getAttribute(KEY_ATTRIBUTE);
  if (liveKey !== null) return `live_key:${stableKey(element, limits)}`;
  return id === null ? null : `id:${id}`;
}

function externalIdentities(root: Element, limits: MorphLimits): ReadonlyMap<string, Element> {
  const identities = new Map<string, Element>();
  const stack = [root];
  while (stack.length > 0) {
    const element = stack.pop();
    if (element === undefined) break;
    const token = externalIdentity(element, limits);
    if (token !== null) {
      if (identities.has(token) || identities.size >= limits.maxKeys) {
        fail("active_teleport_identity");
      }
      identities.set(token, element);
    }
    stack.push(...element.children);
  }
  return identities;
}

function identityLabel(kind: IdentityPlan["entries"][number]["kind"], value: string): string {
  if (kind === "live_key") return value;
  return kind === "id" ? `#${value}` : `island:${value}`;
}

export function planMorphControls(
  currentRoot: Element,
  replacementRoot: Element,
  identity: IdentityPlan,
  limits: MorphLimits,
  teleports?: TeleportTargetPort,
): MorphControlPlan {
  const current = scanControls(currentRoot, limits);
  const replacement = scanControls(replacementRoot, limits);
  const activeElements = new Set<Element>();
  for (const active of teleports?.active?.(currentRoot) ?? []) {
    const control = controlFor(active.node, limits);
    const existing = current.get(active.key);
    if (
      control?.kind !== "teleport" ||
      control.key !== active.key ||
      control.target !== active.target ||
      (existing !== undefined &&
        (existing.element !== active.node || !sameControl(existing.control, control)))
    ) {
      fail("active_teleport_invalid");
    }
    if (existing === undefined) {
      current.set(active.key, Object.freeze({ control, element: active.node }));
    }
    activeElements.add(active.node);
  }
  const identityByKey = new Map(
    identity.entries
      .filter(({ kind }) => kind === "live_key")
      .map((entry) => [entry.value, entry] as const),
  );
  const keys = new Set([...current.keys(), ...replacement.keys()]);
  const bindings: MorphControlBinding[] = [];
  const byKey = new Map<string, MorphControl>();
  const byCurrent = new Map<Element, MorphControl>();
  const byReplacement = new Map<Element, MorphControl>();
  const teleportTargets = new Map<string, Element>();
  for (const key of keys) {
    const currentCandidate = current.get(key);
    const replacementCandidate = replacement.get(key);
    const control = replacementCandidate?.control ?? currentCandidate?.control ?? fail("missing");
    if (
      currentCandidate !== undefined &&
      replacementCandidate !== undefined &&
      !sameControl(currentCandidate.control, replacementCandidate.control)
    ) {
      fail("drift");
    }
    const identityEntry = identityByKey.get(key);
    if (
      (currentCandidate !== undefined &&
        replacementCandidate === undefined &&
        identityEntry?.replacement !== null &&
        identityEntry?.replacement !== undefined) ||
      (currentCandidate === undefined &&
        replacementCandidate !== undefined &&
        identityEntry?.current !== null &&
        identityEntry?.current !== undefined)
    ) {
      fail("drift");
    }
    if (
      identityEntry === undefined &&
      !activeElements.has(currentCandidate?.element ?? currentRoot)
    ) {
      fail("identity_missing");
    }
    if (
      currentCandidate !== undefined &&
      identityEntry?.current !== currentCandidate.element &&
      !activeElements.has(currentCandidate.element)
    ) {
      fail("current_identity");
    }
    if (
      replacementCandidate !== undefined &&
      identityEntry?.replacement !== replacementCandidate.element
    ) {
      fail("replacement_identity");
    }
    if (currentCandidate !== undefined) byCurrent.set(currentCandidate.element, control);
    if (replacementCandidate !== undefined)
      byReplacement.set(replacementCandidate.element, control);
    byKey.set(key, control);
    if (control.kind === "teleport") {
      const target = teleports?.resolve(control.target, currentRoot) ?? null;
      const currentElement = currentCandidate?.element;
      const replacementElement = replacementCandidate?.element;
      if (
        target?.ownerDocument !== currentRoot.ownerDocument ||
        (currentElement === undefined ? false : containedBy(target, currentElement)) ||
        (replacementElement === undefined ? false : containedBy(target, replacementElement))
      ) {
        fail("target_invalid");
      }
      teleportTargets.set(key, target);
    }
    bindings.push(
      Object.freeze({
        control,
        current: currentCandidate?.element ?? null,
        replacement: replacementCandidate?.element ?? null,
      }),
    );
  }
  return Object.freeze({
    bindings: Object.freeze(bindings),
    byKey,
    byCurrent,
    byReplacement,
    teleportTargets,
  });
}

export function reconcileMorphControlIdentity(
  identity: IdentityPlan,
  controls: MorphControlPlan,
  currentRoot: Element,
  limits: MorphLimits,
): IdentityPlan {
  const entries = [...identity.entries];
  const inserted = new Set(identity.inserted);
  const removed = new Set(identity.removed);
  for (const binding of controls.bindings) {
    if (
      binding.control.kind !== "teleport" ||
      binding.current === null ||
      containedBy(binding.current, currentRoot)
    ) {
      continue;
    }
    const active = externalIdentities(binding.current, limits);
    for (const [token, element] of active) {
      const existing = entries.find((entry) => entry.token === token);
      if (
        existing?.current !== null &&
        existing?.current !== undefined &&
        existing.current !== element
      ) {
        fail("active_teleport_identity");
      }
    }
    if (binding.replacement !== null) {
      for (const [entryIndex, existing] of entries.entries()) {
        if (
          existing.current !== null ||
          existing.replacement === null ||
          !containedBy(existing.replacement, binding.replacement)
        ) {
          continue;
        }
        const current = active.get(existing.token);
        if (current === undefined) continue;
        entries[entryIndex] = Object.freeze({
          ...existing,
          current,
          currentPosition: `teleport:${binding.control.target}/${existing.token}`,
        });
        inserted.delete(identityLabel(existing.kind, existing.value));
      }
    }
    const index = entries.findIndex(
      ({ kind, value }) => kind === "live_key" && value === binding.control.key,
    );
    const prior = index < 0 ? undefined : entries[index];
    if (prior?.current !== binding.current) {
      const entry = Object.freeze({
        current: binding.current,
        currentPosition: `teleport:${binding.control.target}`,
        kind: "live_key" as const,
        replacement: binding.replacement,
        replacementPosition: prior?.replacementPosition ?? null,
        token: `live_key:${binding.control.key}`,
        value: binding.control.key,
      });
      if (index < 0) entries.push(entry);
      else entries[index] = entry;
    }
    inserted.delete(binding.control.key);
    if (binding.replacement === null) removed.add(binding.control.key);
  }
  return Object.freeze({
    entries: Object.freeze(entries),
    inserted: Object.freeze([...inserted]),
    moved: identity.moved,
    nestedCurrentRoots: identity.nestedCurrentRoots,
    nestedReplacementRoots: identity.nestedReplacementRoots,
    removed: Object.freeze([...removed]),
  });
}
