import { limitBreach, utf8Length, type LiveLimitBreach } from "../limits.js";
import type { MorphLimits } from "./types.js";

const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
const PROHIBITED_ELEMENTS = new Set(["applet", "base", "embed", "iframe", "object", "script"]);

export interface MorphHtmlParser {
  parseFromString(source: string, type: "text/html"): Document;
}

export class MorphHtmlError extends Error {
  readonly liveLimit: LiveLimitBreach | null;

  constructor(
    readonly detail: string,
    breach: LiveLimitBreach | null = null,
  ) {
    super(breach === null ? "morph_html_invalid" : `${breach.message} (morph_html_invalid)`);
    this.name = "MorphHtmlError";
    this.liveLimit = breach;
  }
}

function fail(detail: string): never {
  throw new MorphHtmlError(detail);
}

function overLimit(detail: string, breach: LiveLimitBreach): never {
  throw new MorphHtmlError(detail, breach);
}

function isElement(node: Node): node is Element {
  return node.nodeType === 1;
}

function parserFor(document: Document): MorphHtmlParser {
  const Parser = document.defaultView?.DOMParser;
  if (Parser === undefined) return fail("parser_unavailable");
  return new Parser();
}

function prohibited(element: Element): boolean {
  const name = element.localName.toLowerCase();
  if (PROHIBITED_ELEMENTS.has(name)) return true;
  if (name === "meta" && element.getAttribute("http-equiv")?.toLowerCase() === "refresh") {
    return true;
  }
  for (const attribute of element.attributes) {
    const attributeName = attribute.name.toLowerCase();
    if (
      attributeName.startsWith("on") ||
      attributeName === "srcdoc" ||
      attributeName.startsWith("data-suprnova-live-internal-")
    ) {
      return true;
    }
  }
  return false;
}

function validateTree(document: Document, root: Element, limits: MorphLimits): void {
  let nodes = 0;
  let attributes = 0;
  const stack: (readonly [Node, number])[] = [[root, 1]];
  while (stack.length > 0) {
    const [node, depth] = stack.pop() ?? fail("tree");
    nodes += 1;
    if (nodes > limits.maxNodes) {
      overLimit("node_limit", limitBreach("morphMaxNodes", nodes, limits.maxNodes));
    }
    if (depth > limits.maxDepth) {
      overLimit("depth_limit", limitBreach("morphMaxDepth", depth, limits.maxDepth));
    }
    if (node.ownerDocument !== document) fail("cross_document_node");
    if (node.nodeType !== 1 && node.nodeType !== 3 && node.nodeType !== 8) {
      fail("node_type");
    }
    if (isElement(node)) {
      if (node.localName.toLowerCase() === "parsererror" || prohibited(node)) {
        fail("prohibited_structure");
      }
      if (node.attributes.length > limits.maxAttributesPerElement) {
        overLimit(
          "attribute_limit",
          limitBreach(
            "morphMaxAttributesPerElement",
            node.attributes.length,
            limits.maxAttributesPerElement,
          ),
        );
      }
      attributes += node.attributes.length;
      if (attributes > limits.maxAttributes) {
        overLimit(
          "attribute_limit",
          limitBreach("morphMaxAttributes", attributes, limits.maxAttributes),
        );
      }
    }
    const children = [...node.childNodes];
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const child = children[index];
      if (child !== undefined) stack.push([child, depth + 1]);
    }
  }
}

export function parseMorphHtml(
  ownerDocument: Document,
  html: string,
  limits: MorphLimits,
  parser: MorphHtmlParser = parserFor(ownerDocument),
): HTMLElement {
  const htmlBytes = utf8Length(html);
  if (htmlBytes === 0) fail("empty");
  if (htmlBytes > limits.maxHtmlBytes) {
    overLimit("byte_limit", limitBreach("maxHtmlBytes", htmlBytes, limits.maxHtmlBytes));
  }
  let parsed: Document;
  try {
    parsed = parser.parseFromString(html, "text/html");
  } catch {
    return fail("parser_failure");
  }
  const roots = [...parsed.body.children];
  const extraneous = [...parsed.body.childNodes].some(
    (node) =>
      node.nodeType !== 1 && (node.nodeType !== 3 || (node.textContent ?? "").trim() !== ""),
  );
  if (extraneous || roots.length !== 1) fail("root_count");
  const root = roots[0];
  if (root?.namespaceURI !== HTML_NAMESPACE) fail("root_type");
  validateTree(parsed, root, limits);
  return root as HTMLElement;
}
