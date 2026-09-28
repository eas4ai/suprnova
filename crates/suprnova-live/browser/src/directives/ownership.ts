import { parseDirective } from "./parser.js";
import type { ParsedDirective } from "./types.js";
import { ISLAND_ROOT_SELECTOR } from "../islands/metadata.js";
import type { IslandRecord } from "../islands/record.js";
import type { DelegatedEventPhase } from "./modifiers.js";

const MAX_SCANNED_ELEMENTS_PER_ISLAND = 4_096;
const MAX_DIRECTIVES_PER_ISLAND = 2_048;

export interface OwnedDirective {
  readonly attributeName: string;
  readonly directive: ParsedDirective;
  readonly element: Element;
  readonly island: IslandRecord;
}

function asElement(node: EventTarget | Node | null): Element | null {
  return node !== null && "nodeType" in node && node.nodeType === 1 ? (node as Element) : null;
}

function openShadowRoot(element: Element): ShadowRoot | null {
  return "shadowRoot" in element ? element.shadowRoot : null;
}

function* walkElements(
  node: Node,
  islandRoot: Element,
  ignoredRoots: WeakSet<Element>,
): Generator<Element> {
  const element = asElement(node);
  if (element !== null) {
    if (element !== islandRoot && element.matches(ISLAND_ROOT_SELECTOR)) return;
    yield element;
    if (ignoredRoots.has(element)) return;
    for (const child of element.children) yield* walkElements(child, islandRoot, ignoredRoots);
    const shadow = openShadowRoot(element);
    if (shadow !== null) {
      for (const child of shadow.children) yield* walkElements(child, islandRoot, ignoredRoots);
    }
    return;
  }
  if (node.nodeType === 11) {
    for (const child of (node as DocumentFragment).children) {
      yield* walkElements(child, islandRoot, ignoredRoots);
    }
  }
}

function liveAttributes(element: Element): readonly Attr[] {
  return [...element.attributes].filter((attribute) => attribute.name.startsWith("live:"));
}

export class DirectiveOwnership {
  readonly #byElement = new WeakMap<Element, readonly OwnedDirective[]>();
  readonly #ownerByElement = new WeakMap<Element, IslandRecord>();
  readonly #validated = new WeakSet<Element>();
  readonly #roots = new WeakMap<Element, IslandRecord>();
  readonly #ignoredRoots = new WeakSet<Element>();
  readonly #byRecord = new Map<IslandRecord, OwnedDirective[]>();
  readonly #elementsByRecord = new Map<IslandRecord, Element[]>();

  connect(record: IslandRecord): readonly OwnedDirective[] {
    this.#roots.set(record.element, record);
    this.#byRecord.set(record, []);
    this.#elementsByRecord.set(record, []);
    const scanned = this.#scan(record, record.element);
    record.onDispose(() => {
      this.retire(record);
    });
    return scanned;
  }

  scanInsertion(record: IslandRecord, node: Node, trusted: boolean): readonly OwnedDirective[] {
    return trusted ? this.#scan(record, node) : [];
  }

  directives(record: IslandRecord): readonly OwnedDirective[] {
    return this.#byRecord.get(record) ?? [];
  }

  ownerForNode(node: Node): IslandRecord | null {
    let current: Node | null = node;
    while (current !== null) {
      const element = asElement(current);
      if (element !== null) {
        const owner = this.#ownerByElement.get(element);
        if (owner !== undefined) return owner;
        const root = this.#roots.get(element);
        if (root !== undefined) return root;
      }
      if (current.parentNode !== null) {
        current = current.parentNode;
        continue;
      }
      const root = current.getRootNode();
      current = root instanceof ShadowRoot ? root.host : null;
    }
    return null;
  }

  resolve(
    path: readonly EventTarget[],
    eventType: string,
    phase: DelegatedEventPhase,
  ): OwnedDirective | null {
    for (const target of path) {
      const element = asElement(target);
      if (element === null) continue;
      const directives = this.#byElement.get(element) ?? [];
      const owned = directives.find(
        (candidate) =>
          candidate.directive.name === eventType &&
          (candidate.directive.modifiers.includes("capture") ? "capture" : "bubble") === phase,
      );
      if (owned !== undefined) return owned;
      if (this.#roots.has(element)) return null;
    }
    return null;
  }

  resolveNamed(path: readonly EventTarget[], names: ReadonlySet<string>): OwnedDirective | null {
    for (const target of path) {
      const element = asElement(target);
      if (element === null) continue;
      const owned = (this.#byElement.get(element) ?? []).find((candidate) =>
        names.has(candidate.directive.name),
      );
      if (owned !== undefined) return owned;
      if (this.#roots.has(element)) return null;
    }
    return null;
  }

  retire(record: IslandRecord): void {
    this.#roots.delete(record.element);
    for (const element of this.#elementsByRecord.get(record) ?? []) {
      this.#byElement.delete(element);
      this.#ownerByElement.delete(element);
      this.#validated.delete(element);
    }
    this.#elementsByRecord.delete(record);
    this.#byRecord.delete(record);
  }

  retireSubtree(record: IslandRecord, node: Node): void {
    const recordDirectives = this.#byRecord.get(record);
    const recordElements = this.#elementsByRecord.get(record);
    if (recordDirectives === undefined || recordElements === undefined) return;
    for (const element of walkElements(node, record.element, this.#ignoredRoots)) {
      if (this.#ownerByElement.get(element) !== record) continue;
      const directives = this.#byElement.get(element) ?? [];
      for (const directive of directives) {
        const index = recordDirectives.indexOf(directive);
        if (index >= 0) recordDirectives.splice(index, 1);
      }
      const elementIndex = recordElements.indexOf(element);
      if (elementIndex >= 0) recordElements.splice(elementIndex, 1);
      this.#byElement.delete(element);
      this.#ownerByElement.delete(element);
      this.#validated.delete(element);
    }
  }

  #scan(record: IslandRecord, node: Node): readonly OwnedDirective[] {
    const recordDirectives = this.#byRecord.get(record);
    const recordElements = this.#elementsByRecord.get(record);
    if (recordDirectives === undefined || recordElements === undefined) return [];
    let ancestor = node.parentElement;
    while (ancestor !== null && ancestor !== record.element) {
      if (this.#ignoredRoots.has(ancestor)) return [];
      ancestor = ancestor.parentElement;
    }
    const added: OwnedDirective[] = [];
    for (const element of walkElements(node, record.element, this.#ignoredRoots)) {
      if (this.#validated.has(element)) continue;
      if (recordElements.length >= MAX_SCANNED_ELEMENTS_PER_ISLAND) break;
      this.#validated.add(element);
      this.#ownerByElement.set(element, record);
      recordElements.push(element);
      const attributes = liveAttributes(element);
      const names = attributes.map((attribute) => attribute.name);
      const owned: OwnedDirective[] = [];
      for (const attribute of attributes) {
        if (recordDirectives.length >= MAX_DIRECTIVES_PER_ISLAND) break;
        const parsed = parseDirective(attribute.name, attribute.value, names);
        if (!parsed.ok) continue;
        const candidate = Object.freeze({
          attributeName: attribute.name,
          directive: parsed,
          element,
          island: record,
        });
        owned.push(candidate);
        added.push(candidate);
        recordDirectives.push(candidate);
      }
      if (owned.some(({ directive }) => directive.name === "ignore")) {
        this.#ignoredRoots.add(element);
      }
      this.#byElement.set(element, Object.freeze(owned));
    }
    return Object.freeze(added);
  }
}
