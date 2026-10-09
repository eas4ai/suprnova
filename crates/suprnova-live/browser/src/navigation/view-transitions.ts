import type { NativeNavigationIntent } from "./eligibility.js";

export const DOCUMENT_TRANSITION_ATTRIBUTE = "data-suprnova-live-document-transition";

const MAX_DOCUMENT_TRANSITIONS = 64;
const TRANSITION_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/u;

export type DocumentTransitionDisposition = "enhanced" | "native";

export interface DocumentTransitionCapabilities {
  prefersReducedMotion(): boolean;
  supportsViewTransitions(): boolean;
}

interface AppliedTransitionName {
  readonly element: HTMLElement;
  readonly previous: string;
}

function checkedNames(
  document: Document,
): readonly Readonly<{ element: HTMLElement; name: string }>[] {
  const elements = document.querySelectorAll(`[${DOCUMENT_TRANSITION_ATTRIBUTE}]`);
  if (elements.length > MAX_DOCUMENT_TRANSITIONS) return [];
  const seen = new Set<string>();
  const checked: Readonly<{ element: HTMLElement; name: string }>[] = [];
  const HTMLElementConstructor = document.defaultView?.HTMLElement;
  if (HTMLElementConstructor === undefined) return [];
  for (const element of elements) {
    if (!(element instanceof HTMLElementConstructor)) return [];
    const name = element.getAttribute(DOCUMENT_TRANSITION_ATTRIBUTE) ?? "";
    if (name === "none" || name.startsWith("--") || !TRANSITION_NAME.test(name) || seen.has(name)) {
      return [];
    }
    seen.add(name);
    checked.push(Object.freeze({ element, name }));
  }
  return checked;
}

export class DocumentViewTransitions {
  readonly #document: Document;
  readonly #capabilities: DocumentTransitionCapabilities;
  readonly #applied: AppliedTransitionName[] = [];

  constructor(document: Document, capabilities: DocumentTransitionCapabilities) {
    this.#document = document;
    this.#capabilities = capabilities;
  }

  start(): DocumentTransitionDisposition {
    this.cancel();
    if (
      !this.#capabilities.supportsViewTransitions() ||
      this.#capabilities.prefersReducedMotion()
    ) {
      return "native";
    }
    const declarations = checkedNames(this.#document);
    if (declarations.length === 0) return "native";
    try {
      for (const declaration of declarations) {
        const previous = declaration.element.style.getPropertyValue("view-transition-name");
        declaration.element.style.setProperty(
          "view-transition-name",
          `suprnova-document-${declaration.name}`,
        );
        this.#applied.push({ element: declaration.element, previous });
      }
      return "enhanced";
    } catch {
      this.cancel();
      return "native";
    }
  }

  prepare(intent: NativeNavigationIntent): DocumentTransitionDisposition {
    if (intent.transitionName === null) return "native";
    return this.start();
  }

  cancel(): void {
    for (const applied of this.#applied.splice(0).reverse()) {
      try {
        if (applied.previous.length === 0) {
          applied.element.style.removeProperty("view-transition-name");
        } else {
          applied.element.style.setProperty("view-transition-name", applied.previous);
        }
      } catch {
        try {
          applied.element.style.removeProperty("view-transition-name");
        } catch {
          // Cleanup is best effort; native navigation remains authoritative.
        }
      }
    }
  }
}
