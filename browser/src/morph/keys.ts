import type { IdentityPlan, MorphIdentityEntry, MorphIdentityKind, MorphLimits } from "./types.js";

const ISLAND_ATTRIBUTE = "data-suprnova-live-island";
const KEY_ATTRIBUTE = "data-suprnova-live-key";
const DOCUMENT_KEY_ATTRIBUTE = "data-suprnova-live-document-key";
const STATUS_ATTRIBUTE = "data-suprnova-live-status";
const SAFE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;

interface TreeIdentity {
  readonly kind: MorphIdentityKind;
  readonly value: string;
  readonly token: string;
  readonly element: Element;
  readonly position: string;
  readonly nested: boolean;
}

function fail(detail: string): never {
  throw new Error(`morph_identity_${detail}`);
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function validKey(value: string | null, limits: MorphLimits): string {
  if (value === null || !SAFE_KEY.test(value) || utf8Length(value) > limits.maxKeyBytes) {
    return fail("key_invalid");
  }
  return value;
}

function identityLabel(kind: MorphIdentityKind, value: string): string {
  if (kind === "live_key") return value;
  return kind === "id" ? `#${value}` : `island:${value}`;
}

function elementPosition(element: Element, parentIdentity: string): string {
  const parent = element.parentElement;
  if (parent === null) return `${parentIdentity}/0`;
  return `${parentIdentity}/${[...parent.children].indexOf(element).toString(10)}`;
}

function scanOwnedTree(root: Element, limits: MorphLimits): readonly TreeIdentity[] {
  const identities: TreeIdentity[] = [];
  const seen = new Set<string>();
  const seenIds = new Set<string>();
  const stack: (readonly [Element, string])[] = [];
  const rootChildren = [...root.children];
  for (let index = rootChildren.length - 1; index >= 0; index -= 1) {
    const child = rootChildren[index];
    if (child !== undefined) stack.push([child, "root"]);
  }
  while (stack.length > 0) {
    const [element, parentIdentity] = stack.pop() ?? fail("tree");
    const nested = element.hasAttribute(ISLAND_ATTRIBUTE);
    const liveKey = element.getAttribute(KEY_ATTRIBUTE);
    const id = element.getAttribute("id");
    if (id !== null) {
      const validatedId = validKey(id, limits);
      if (seenIds.has(validatedId)) fail("duplicate_key");
      seenIds.add(validatedId);
    }
    let kind: MorphIdentityKind | null = null;
    let value: string | null = null;
    if (nested) {
      kind = "nested_island";
      value = validKey(element.getAttribute(DOCUMENT_KEY_ATTRIBUTE), limits);
      if (liveKey !== null && validKey(liveKey, limits) !== value) return fail("ambiguous_key");
    } else if (liveKey !== null) {
      kind = "live_key";
      value = validKey(liveKey, limits);
    } else if (id !== null) {
      kind = "id";
      value = validKey(id, limits);
    }
    let childParent = parentIdentity;
    if (kind !== null && value !== null) {
      const token = `${kind}:${value}`;
      if (seen.has(token)) fail("duplicate_key");
      if (identities.length >= limits.maxKeys) fail("key_limit");
      seen.add(token);
      identities.push(
        Object.freeze({
          element,
          kind,
          nested,
          position: elementPosition(element, parentIdentity),
          token,
          value,
        }),
      );
      childParent = token;
    }
    if (nested) continue;
    const children = [...element.children];
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const child = children[index];
      if (child !== undefined) stack.push([child, childParent]);
    }
  }
  return Object.freeze(identities);
}

function engineMetadata(element: Element): string {
  return [...element.attributes]
    .filter(({ name }) => name.startsWith("data-suprnova-live-") && name !== STATUS_ATTRIBUTE)
    .map(({ name, value }) => `${name}\u0000${value}`)
    .sort()
    .join("\u0001");
}

function compatible(current: TreeIdentity, replacement: TreeIdentity): void {
  if (
    current.element.namespaceURI !== replacement.element.namespaceURI ||
    current.element.localName !== replacement.element.localName
  ) {
    fail("ambiguous_key");
  }
  if (current.nested && engineMetadata(current.element) !== engineMetadata(replacement.element)) {
    fail("nested_owner_escape");
  }
}

export function planMorphIdentity(
  currentRoot: Element,
  replacementRoot: Element,
  limits: MorphLimits,
): IdentityPlan {
  const current = scanOwnedTree(currentRoot, limits);
  const replacement = scanOwnedTree(replacementRoot, limits);
  const currentByToken = new Map(current.map((entry) => [entry.token, entry]));
  const replacementByToken = new Map(replacement.map((entry) => [entry.token, entry]));
  const tokens = new Set([...currentByToken.keys(), ...replacementByToken.keys()]);
  const entries: MorphIdentityEntry[] = [];
  const moved: string[] = [];
  const inserted: string[] = [];
  const removed: string[] = [];
  for (const token of tokens) {
    const oldEntry = currentByToken.get(token) ?? null;
    const newEntry = replacementByToken.get(token) ?? null;
    if (oldEntry !== null && newEntry !== null) compatible(oldEntry, newEntry);
    const kind = oldEntry?.kind ?? newEntry?.kind ?? fail("tree");
    const value = oldEntry?.value ?? newEntry?.value ?? fail("tree");
    const label = identityLabel(kind, value);
    if (oldEntry === null) inserted.push(label);
    else if (newEntry === null) removed.push(label);
    else if (oldEntry.position !== newEntry.position) moved.push(label);
    entries.push(
      Object.freeze({
        current: oldEntry?.element ?? null,
        currentPosition: oldEntry?.position ?? null,
        kind,
        replacement: newEntry?.element ?? null,
        replacementPosition: newEntry?.position ?? null,
        token,
        value,
      }),
    );
  }
  return Object.freeze({
    entries: Object.freeze(entries),
    inserted: Object.freeze(inserted),
    moved: Object.freeze(moved),
    nestedCurrentRoots: new Set(
      current.filter(({ nested }) => nested).map(({ element }) => element),
    ),
    nestedReplacementRoots: new Set(
      replacement.filter(({ nested }) => nested).map(({ element }) => element),
    ),
    removed: Object.freeze(removed),
  });
}
