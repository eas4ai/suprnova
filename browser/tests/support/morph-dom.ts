import type { MorphAuthority, MorphLimits } from "../../src/morph/types.js";
import { DEFAULT_MORPH_LIMITS } from "../../src/morph/limits.js";

const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";

export class FakeNode {
  readonly nodeType: number;
  ownerDocument: FakeDocument;
  parentElement: FakeElement | null = null;
  readonly childNodes: FakeNode[] = [];
  isConnected = false;
  #text: string;

  constructor(nodeType: number, ownerDocument: FakeDocument, text = "") {
    this.nodeType = nodeType;
    this.ownerDocument = ownerDocument;
    this.#text = text;
  }

  get textContent(): string {
    if (this.nodeType === 3 || this.nodeType === 8) return this.#text;
    return this.childNodes.map(({ textContent }) => textContent).join("");
  }
}

export class FakeElement extends FakeNode {
  readonly localName: string;
  readonly namespaceURI: string;
  readonly #attributes = new Map<string, string>();

  constructor(
    ownerDocument: FakeDocument,
    name: string,
    attributes: Readonly<Record<string, string>> = {},
    children: readonly FakeNode[] = [],
    namespaceURI = HTML_NAMESPACE,
  ) {
    super(1, ownerDocument);
    this.localName = name.toLowerCase();
    this.namespaceURI = namespaceURI;
    for (const [key, value] of Object.entries(attributes)) this.#attributes.set(key, value);
    for (const child of children) this.append(child);
  }

  get tagName(): string {
    return this.localName.toUpperCase();
  }

  get attributes(): readonly Readonly<{ name: string; value: string }>[] {
    return [...this.#attributes].map(([name, value]) => Object.freeze({ name, value }));
  }

  get children(): readonly FakeElement[] {
    return this.childNodes.filter((node): node is FakeElement => node.nodeType === 1);
  }

  append(child: FakeNode): void {
    child.parentElement = this;
    this.childNodes.push(child);
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.#attributes.has(name);
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }

  removeAttribute(name: string): void {
    this.#attributes.delete(name);
  }
}

export class FakeDocument {
  readonly body: FakeElement;
  readonly defaultView = null;

  constructor() {
    this.body = new FakeElement(this, "body");
    this.body.isConnected = true;
  }
}

function connect(node: FakeNode, connected: boolean): void {
  node.isConnected = connected;
  for (const child of node.childNodes) connect(child, connected);
}

function rehome(node: FakeNode, document: FakeDocument): void {
  node.ownerDocument = document;
  for (const child of node.childNodes) rehome(child, document);
}

export const AUTHORITY: MorphAuthority = Object.freeze({
  component: "catalog.search",
  documentKey: "catalog-page",
  encodedSnapshot: "successor-snapshot",
  instanceId: "AAECAwQFBgcICQoLDA0ODw",
  slot: "primary",
  successorRevision: 8n,
});

export function rootAttributes(
  revision: string,
  snapshot: string,
  overrides: Readonly<Record<string, string>> = {},
): Readonly<Record<string, string>> {
  return {
    "data-suprnova-live-component": AUTHORITY.component,
    "data-suprnova-live-contract": "1",
    "data-suprnova-live-document-key": AUTHORITY.documentKey,
    "data-suprnova-live-instance": AUTHORITY.instanceId,
    "data-suprnova-live-island": "",
    "data-suprnova-live-lazy-complete": "true",
    "data-suprnova-live-protocol-min": "2",
    "data-suprnova-live-revision": revision,
    "data-suprnova-live-root": AUTHORITY.slot,
    "data-suprnova-live-slot": AUTHORITY.slot,
    "data-suprnova-live-snapshot": snapshot,
    "data-suprnova-live-snapshot-kind": "instance",
    ...overrides,
  };
}

export function element(
  document: FakeDocument,
  name: string,
  attributes: Readonly<Record<string, string>> = {},
  children: readonly FakeNode[] = [],
): FakeElement {
  return new FakeElement(document, name, attributes, children);
}

export function text(document: FakeDocument, value: string): FakeNode {
  return new FakeNode(3, document, value);
}

export interface MorphFixture {
  readonly authority: MorphAuthority;
  readonly currentDocument: FakeDocument;
  readonly currentRoot: FakeElement;
  readonly limits: MorphLimits;
  readonly parser: Readonly<{ parseFromString(source: string, type: "text/html"): Document }>;
  readonly replacementDocument: FakeDocument;
  readonly replacementRoot: FakeElement;
}

export function morphFixture(
  options: {
    readonly currentChildren?: readonly FakeNode[];
    readonly currentOverrides?: Readonly<Record<string, string>>;
    readonly replacementChildren?: readonly FakeNode[];
    readonly replacementOverrides?: Readonly<Record<string, string>>;
    readonly limits?: MorphLimits;
  } = {},
): MorphFixture {
  const currentDocument = new FakeDocument();
  const replacementDocument = new FakeDocument();
  const currentRoot = element(
    currentDocument,
    "section",
    rootAttributes("7", "current-snapshot", options.currentOverrides),
    options.currentChildren,
  );
  const replacementRoot = element(
    replacementDocument,
    "section",
    rootAttributes("8", AUTHORITY.encodedSnapshot, options.replacementOverrides),
    options.replacementChildren,
  );
  currentDocument.body.append(currentRoot);
  replacementDocument.body.append(replacementRoot);
  rehome(currentRoot, currentDocument);
  rehome(replacementRoot, replacementDocument);
  connect(currentDocument.body, true);
  connect(replacementDocument.body, true);
  return {
    authority: AUTHORITY,
    currentDocument,
    currentRoot,
    limits: options.limits ?? DEFAULT_MORPH_LIMITS,
    parser: {
      parseFromString: () => replacementDocument as unknown as Document,
    },
    replacementDocument,
    replacementRoot,
  };
}

export function asElement(value: FakeElement): HTMLElement {
  return value as unknown as HTMLElement;
}

export function withLimits(overrides: Partial<MorphLimits>): MorphLimits {
  return Object.freeze({ ...DEFAULT_MORPH_LIMITS, ...overrides });
}
