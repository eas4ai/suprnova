/**
 * A small DOM for driving the whole `DocumentRuntime` in Node: elements with
 * attributes and children, the selectors the runtime asks for, listeners on
 * the document and the window, and the constructors its `instanceof` checks
 * name. It covers what the agent tool runtime tests exercise, not the
 * platform.
 */
import { vi } from "vitest";

type Listener = (event: unknown) => void;

/**
 * The constructor of every element kind this DOM never builds, buttons,
 * forms, anchors and shadow roots, so an `instanceof` check of one fails.
 */
class NotThisElement {
  readonly kind = "absent";
}

class Listeners {
  readonly #byType = new Map<string, Set<Listener>>();

  add(type: string, listener: unknown): void {
    if (typeof listener !== "function") return;
    const set = this.#byType.get(type) ?? new Set<Listener>();
    set.add(listener as Listener);
    this.#byType.set(type, set);
  }

  remove(type: string, listener: unknown): void {
    this.#byType.get(type)?.delete(listener as Listener);
  }

  dispatch(type: string, event: unknown): void {
    for (const listener of [...(this.#byType.get(type) ?? [])]) listener(event);
  }
}

/** The topmost ancestor of `node`, `node` itself when it has no parent. */
function topOf(node: RuntimeNode): RuntimeNode {
  return node.parentNode === null ? node : topOf(node.parentNode);
}

export class RuntimeNode {
  readonly nodeType: number;
  ownerDocument: RuntimeDocument;
  parentNode: RuntimeElement | null = null;
  readonly childNodes: RuntimeNode[] = [];
  #text: string;

  constructor(nodeType: number, ownerDocument: RuntimeDocument, text = "") {
    this.nodeType = nodeType;
    this.ownerDocument = ownerDocument;
    this.#text = text;
  }

  get parentElement(): RuntimeElement | null {
    return this.parentNode;
  }

  get isConnected(): boolean {
    return topOf(this) === this.ownerDocument.documentElement;
  }

  get textContent(): string {
    if (this.nodeType === 3) return this.#text;
    return this.childNodes.map((node) => node.textContent).join("");
  }

  getRootNode(): unknown {
    return this.isConnected ? this.ownerDocument : this;
  }

  contains(other: RuntimeNode | null): boolean {
    let node: RuntimeNode | null = other;
    while (node !== null) {
      if (node === this) return true;
      node = node.parentNode;
    }
    return false;
  }

  remove(): void {
    const parent = this.parentNode;
    if (parent === null) return;
    parent.childNodes.splice(parent.childNodes.indexOf(this), 1);
    this.parentNode = null;
    this.ownerDocument.mutated([], [this], parent);
  }
}

/** One `[name]` or `[name="value"]` test, or a tag name, or `*`. */
function matchesSimple(element: RuntimeElement, selector: string): boolean {
  const trimmed = selector.trim();
  if (trimmed === "*") return true;
  const attribute = /^\[([^\]=~^$*|]+)(?:="([^"]*)")?\]$/u.exec(trimmed);
  if (attribute !== null) {
    const name = attribute[1] ?? "";
    const value = attribute[2];
    return value === undefined ? element.hasAttribute(name) : element.getAttribute(name) === value;
  }
  if (/^[a-z][a-z0-9-]*$/u.test(trimmed)) return element.localName === trimmed;
  return false;
}

export class RuntimeElement extends RuntimeNode {
  readonly localName: string;
  readonly namespaceURI = "http://www.w3.org/1999/xhtml";
  readonly shadowRoot = null;
  readonly #attributes = new Map<string, string>();
  readonly #listeners = new Listeners();

  constructor(
    ownerDocument: RuntimeDocument,
    name: string,
    attributes: Readonly<Record<string, string>> = {},
  ) {
    super(1, ownerDocument);
    this.localName = name;
    for (const [key, value] of Object.entries(attributes)) this.#attributes.set(key, value);
  }

  get tagName(): string {
    return this.localName.toUpperCase();
  }

  cloneNode(deep = false): RuntimeElement {
    const clone = new RuntimeElement(
      this.ownerDocument,
      this.localName,
      Object.fromEntries(this.#attributes),
    );
    if (deep) {
      for (const child of this.children) clone.append(child.cloneNode(true));
    }
    return clone;
  }

  get id(): string {
    return this.getAttribute("id") ?? "";
  }

  get attributes(): readonly Readonly<{ name: string; value: string }>[] {
    return [...this.#attributes].map(([name, value]) => Object.freeze({ name, value }));
  }

  get children(): RuntimeElement[] & { item(index: number): RuntimeElement | null } {
    const children = this.childNodes.filter(
      (node): node is RuntimeElement => node instanceof RuntimeElement,
    );
    return Object.assign(children, { item: (index: number) => children[index] ?? null });
  }

  get firstElementChild(): RuntimeElement | null {
    return this.children[0] ?? null;
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  getAttributeNames(): string[] {
    return [...this.#attributes.keys()];
  }

  hasAttribute(name: string): boolean {
    return this.#attributes.has(name);
  }

  hasAttributes(): boolean {
    return this.#attributes.size > 0;
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }

  removeAttribute(name: string): void {
    this.#attributes.delete(name);
  }

  toggleAttribute(name: string, force?: boolean): boolean {
    const on = force ?? !this.hasAttribute(name);
    if (on) this.setAttribute(name, "");
    else this.removeAttribute(name);
    return on;
  }

  append(...children: RuntimeNode[]): void {
    for (const child of children) {
      child.remove();
      child.parentNode = this;
      this.childNodes.push(child);
      this.ownerDocument.mutated([child], [], this);
    }
  }

  matches(selector: string): boolean {
    return selector.split(",").some((part) => matchesSimple(this, part));
  }

  closest(selector: string): RuntimeElement | null {
    if (this.matches(selector)) return this;
    return this.parentNode === null ? null : this.parentNode.closest(selector);
  }

  querySelectorAll(selector: string): RuntimeElement[] {
    const found: RuntimeElement[] = [];
    const visit = (element: RuntimeElement): void => {
      for (const child of element.children) {
        if (child.matches(selector)) found.push(child);
        visit(child);
      }
    };
    visit(this);
    return found;
  }

  querySelector(selector: string): RuntimeElement | null {
    return this.querySelectorAll(selector)[0] ?? null;
  }

  addEventListener(type: string, listener: unknown): void {
    this.#listeners.add(type, listener);
  }

  removeEventListener(type: string, listener: unknown): void {
    this.#listeners.remove(type, listener);
  }

  focus(): void {
    this.ownerDocument.activeElement = this;
  }

  blur(): void {
    if (this.ownerDocument.activeElement === this) this.ownerDocument.activeElement = null;
  }
}

/** A mutation observer the document feeds as its tree changes. */
export class RuntimeMutationObserver {
  readonly #callback: MutationCallback;
  readonly #document: RuntimeDocument;
  observing = false;

  constructor(document: RuntimeDocument, callback: MutationCallback) {
    this.#document = document;
    this.#callback = callback;
    document.observers.add(this);
  }

  observe(): void {
    this.observing = true;
  }

  disconnect(): void {
    this.observing = false;
  }

  takeRecords(): MutationRecord[] {
    return [];
  }

  deliver(records: MutationRecord[]): void {
    if (this.observing) this.#callback(records, this);
  }

  get document(): RuntimeDocument {
    return this.#document;
  }
}

export class RuntimeDocument {
  readonly nodeType = 9;
  readonly documentElement: RuntimeElement;
  readonly head: RuntimeElement;
  readonly body: RuntimeElement;
  readonly baseURI = "https://app.example.test/page";
  readonly readyState = "complete";
  readonly visibilityState = "visible";
  readonly observers = new Set<RuntimeMutationObserver>();
  readonly #listeners = new Listeners();
  readonly defaultView: Record<string, unknown>;
  activeElement: RuntimeElement | null = null;
  modelContext: unknown = undefined;

  constructor() {
    this.documentElement = new RuntimeElement(this, "html");
    this.head = new RuntimeElement(this, "head");
    this.body = new RuntimeElement(this, "body");
    this.documentElement.append(this.head, this.body);
    const windowListeners = new Listeners();
    this.defaultView = {
      location: { href: this.baseURI, hash: "", pathname: "/page", search: "" },
      performance: { getEntriesByType: () => [] },
      navigator: { onLine: true },
      history: { state: null },
      confirm: () => true,
      addEventListener: (type: string, listener: unknown) => {
        windowListeners.add(type, listener);
      },
      removeEventListener: (type: string, listener: unknown) => {
        windowListeners.remove(type, listener);
      },
      HTMLElement: RuntimeElement,
      Element: RuntimeElement,
      HTMLAnchorElement: NotThisElement,
      HTMLFormElement: NotThisElement,
    };
  }

  createElement(name: string): RuntimeElement {
    return new RuntimeElement(this, name);
  }

  querySelectorAll(selector: string): RuntimeElement[] {
    return [
      ...(this.documentElement.matches(selector) ? [this.documentElement] : []),
      ...this.documentElement.querySelectorAll(selector),
    ];
  }

  querySelector(selector: string): RuntimeElement | null {
    return this.querySelectorAll(selector)[0] ?? null;
  }

  getElementById(id: string): RuntimeElement | null {
    return this.querySelector(`[id="${id}"]`);
  }

  addEventListener(type: string, listener: unknown): void {
    this.#listeners.add(type, listener);
  }

  removeEventListener(type: string, listener: unknown): void {
    this.#listeners.remove(type, listener);
  }

  dispatch(type: string, event: unknown): void {
    this.#listeners.dispatch(type, event);
  }

  /** Report a tree change to every observer, as a microtask would. */
  mutated(added: RuntimeNode[], removed: RuntimeNode[], target: RuntimeElement): void {
    if (added.length === 0 && removed.length === 0) return;
    const record = {
      type: "childList",
      target,
      addedNodes: added,
      removedNodes: removed,
      attributeName: null,
    } as unknown as MutationRecord;
    for (const observer of this.observers) observer.deliver([record]);
  }
}

/**
 * Make the runtime's `instanceof` checks see this DOM's elements as
 * elements. Undo with `vi.unstubAllGlobals()`.
 */
export function stubDomGlobals(): void {
  vi.stubGlobal("Node", RuntimeNode);
  vi.stubGlobal("Element", RuntimeElement);
  vi.stubGlobal("HTMLElement", RuntimeElement);
  vi.stubGlobal("HTMLButtonElement", NotThisElement);
  vi.stubGlobal("HTMLInputElement", NotThisElement);
  vi.stubGlobal("HTMLFormElement", NotThisElement);
  vi.stubGlobal("HTMLAnchorElement", NotThisElement);
  vi.stubGlobal("ShadowRoot", NotThisElement);
}
