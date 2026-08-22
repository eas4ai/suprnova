import { parseDirective } from "../directives/parser.js";
import type { ParsedDirective } from "../directives/types.js";
import type { RuntimePorts } from "../runtime/ports.js";
import { NAVIGATION_RUNTIME_LIMITS } from "../runtime/config.js";
import {
  isSameDocumentFragment,
  nativeNavigationIntent,
  type NativeNavigationCandidate,
  type NativeNavigationIntent,
  type NavigationActivation,
} from "./eligibility.js";
import { applyDocumentFocusScroll } from "./focus-scroll.js";
import { DirtyNavigationGuard } from "./guards.js";
import {
  BrowserPrefetchHost,
  PrefetchCoordinator,
  type PrefetchCachePolicy,
  type PrefetchContext,
  type PrefetchVariance,
} from "./prefetch.js";
import { DocumentViewTransitions } from "./view-transitions.js";

const PREFETCH_CACHE_ATTRIBUTE = "data-suprnova-live-prefetch-cache";
const PREFETCH_VARY_ATTRIBUTE = "data-suprnova-live-prefetch-vary";
const PREFETCH_FLASH_ATTRIBUTE = "data-suprnova-live-prefetch-consumes-flash";
const PREFETCH_REDIRECT_ATTRIBUTE = "data-suprnova-live-prefetch-redirect-prone";
const NAVIGATION_TRANSITION_NAME_ATTRIBUTE = "data-suprnova-live-transition-name";

type NativeRuntimeState = "idle" | "running" | "suspended" | "disposed";

interface AnchorContract {
  readonly intent: NativeNavigationIntent;
  readonly candidate: NativeNavigationCandidate;
  readonly prefetch: ParsedDirective | null;
  readonly navigate: ParsedDirective | null;
}

function directive(element: Element, name: "navigate" | "prefetch"): ParsedDirective | null {
  const attributes = Array.from(element.attributes);
  const present = attributes
    .filter((attribute) => attribute.name.startsWith("live:"))
    .map((attribute) => attribute.name);
  for (const attribute of attributes) {
    if (!attribute.name.startsWith(`live:${name}`)) continue;
    const parsed = parseDirective(
      attribute.name,
      attribute.value,
      present.filter((candidate) => candidate !== attribute.name),
    );
    if (parsed.ok && parsed.name === name) return parsed;
  }
  return null;
}

function activation(event: MouseEvent): NavigationActivation {
  return Object.freeze({
    altKey: event.altKey,
    button: event.button,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    shiftKey: event.shiftKey,
  });
}

function requestedPrefetch(
  navigate: ParsedDirective | null,
  prefetch: ParsedDirective | null,
): boolean {
  return (
    prefetch !== null ||
    navigate?.modifiers.some((modifier) => ["hover", "visible", "eager"].includes(modifier)) ===
      true
  );
}

function anchorContract(anchor: HTMLAnchorElement, event?: MouseEvent): AnchorContract | null {
  const window = anchor.ownerDocument.defaultView;
  if (window === null) return null;
  const href = anchor.getAttribute("href");
  if (href === null || href.length === 0) return null;
  const navigate = directive(anchor, "navigate");
  const prefetch = directive(anchor, "prefetch");
  const wantsPrefetch = requestedPrefetch(navigate, prefetch);
  const transitionName =
    navigate?.modifiers.includes("transition") === true
      ? anchor.getAttribute(NAVIGATION_TRANSITION_NAME_ATTRIBUTE)
      : null;
  const candidate: NativeNavigationCandidate = Object.freeze({
    base: new URL(window.location.href),
    target: href,
    method: "GET",
    history: "navigate",
    prefetch: wantsPrefetch ? "link" : "none",
    transitionName,
    source: "anchor",
    ...(event === undefined ? {} : { activation: activation(event) }),
    ...(anchor.hasAttribute("download") ? { download: true } : {}),
    ...(anchor.getAttribute("target") === null
      ? {}
      : { targetContext: anchor.getAttribute("target") }),
  });
  try {
    return Object.freeze({
      candidate,
      intent: nativeNavigationIntent(candidate),
      navigate,
      prefetch,
    });
  } catch {
    return null;
  }
}

function formIntent(
  form: HTMLFormElement,
  submitter: HTMLElement | null,
): NativeNavigationIntent | null {
  const window = form.ownerDocument.defaultView;
  if (window === null) return null;
  const action =
    submitter?.getAttribute("formaction") ?? form.getAttribute("action") ?? window.location.href;
  const rawMethod = (submitter?.getAttribute("formmethod") ?? form.method).toUpperCase();
  if (rawMethod !== "GET" && rawMethod !== "POST") return null;
  const targetContext = submitter?.getAttribute("formtarget") ?? form.getAttribute("target");
  try {
    return nativeNavigationIntent({
      base: new URL(window.location.href),
      target: action,
      method: rawMethod,
      history: "navigate",
      prefetch: "none",
      transitionName: null,
      source: "form",
      ...(targetContext === null ? {} : { targetContext }),
    });
  } catch {
    return null;
  }
}

function saveData(window: Window): boolean {
  const connection: unknown = Reflect.get(window.navigator, "connection");
  return (
    typeof connection === "object" &&
    connection !== null &&
    Reflect.get(connection, "saveData") === true
  );
}

function hidden(anchor: HTMLAnchorElement): boolean {
  const style = anchor.ownerDocument.defaultView?.getComputedStyle(anchor);
  return (
    anchor.hidden !== false ||
    anchor.getAttribute("aria-hidden") === "true" ||
    anchor.closest('[hidden], [inert], [aria-hidden="true"]') !== null ||
    style?.display === "none" ||
    style?.visibility === "hidden"
  );
}

function prefetchContext(anchor: HTMLAnchorElement): PrefetchContext {
  const window = anchor.ownerDocument.defaultView;
  if (window === null) throw new Error("runtime_window_unavailable");
  const rawCache = anchor.getAttribute(PREFETCH_CACHE_ATTRIBUTE);
  const cachePolicy: PrefetchCachePolicy =
    rawCache === "public" || rawCache === "private" || rawCache === "no-store"
      ? rawCache
      : "private";
  const rawVariance =
    anchor
      .getAttribute(PREFETCH_VARY_ATTRIBUTE)
      ?.split(/[\s,]+/u)
      .filter(Boolean) ?? [];
  const validVariance = rawVariance.every((value): value is PrefetchVariance =>
    ["credentials", "tenant", "principal", "locale"].includes(value),
  );
  return Object.freeze({
    cachePolicy,
    consumesFlash: anchor.getAttribute(PREFETCH_FLASH_ATTRIBUTE) === "true",
    current: new URL(window.location.href),
    explicit: validVariance && rawCache !== null,
    hidden: hidden(anchor),
    redirectProne: anchor.getAttribute(PREFETCH_REDIRECT_ATTRIBUTE) === "true",
    saveData: saveData(window),
    variesBy: validVariance ? rawVariance : [],
  });
}

function anchorFromEvent(document: Document, event: Event): HTMLAnchorElement | null {
  const Anchor = document.defaultView?.HTMLAnchorElement;
  if (Anchor === undefined) return null;
  for (const node of event.composedPath()) {
    if (node instanceof Anchor) return node;
  }
  return null;
}

function currentDocumentDeparts(contract: AnchorContract, anchor: HTMLAnchorElement): boolean {
  const target = anchor.getAttribute("target")?.toLowerCase() ?? "";
  return (
    contract.candidate.download !== true &&
    (target === "" || target === "_self") &&
    contract.candidate.activation?.button === 0 &&
    !contract.candidate.activation.altKey &&
    !contract.candidate.activation.ctrlKey &&
    !contract.candidate.activation.metaKey &&
    !contract.candidate.activation.shiftKey &&
    !isSameDocumentFragment(contract.candidate.base, contract.intent.target)
  );
}

function formDepartsCurrentDocument(form: HTMLFormElement, submitter: HTMLElement | null): boolean {
  const target = (submitter?.getAttribute("formtarget") ?? form.getAttribute("target") ?? "")
    .trim()
    .toLowerCase();
  return target === "" || target === "_self";
}

function removedAnchors(document: Document, node: Node): readonly HTMLAnchorElement[] {
  const Anchor = document.defaultView?.HTMLAnchorElement;
  const ElementConstructor = document.defaultView?.Element;
  if (
    Anchor === undefined ||
    ElementConstructor === undefined ||
    !(node instanceof ElementConstructor)
  ) {
    return [];
  }
  const anchors = Array.from(node.querySelectorAll("a"));
  if (node instanceof Anchor) anchors.unshift(node);
  return anchors.slice(0, NAVIGATION_RUNTIME_LIMITS.maxPrefetchTargets);
}

export class NativeDocumentNavigation {
  readonly #document: Document;
  readonly #prefetch: PrefetchCoordinator;
  readonly #guard: DirtyNavigationGuard;
  readonly #transitions: DocumentViewTransitions;
  readonly #keys = new WeakMap<HTMLAnchorElement, string>();
  readonly #visible: IntersectionObserver | null;
  #nextKey = 0;
  #state: NativeRuntimeState = "idle";
  #focusApplied = false;

  constructor(document: Document, ports: RuntimePorts) {
    const window = document.defaultView;
    if (window === null) throw new Error("runtime_window_unavailable");
    this.#document = document;
    this.#prefetch = new PrefetchCoordinator({
      host: new BrowserPrefetchHost(document),
      maxConcurrent: NAVIGATION_RUNTIME_LIMITS.maxConcurrentPrefetches,
    });
    this.#guard = new DirtyNavigationGuard(document, {
      confirm: (message) => window.confirm(message),
      defer: (callback) => {
        ports.scheduler.timeout(callback, 0);
      },
    });
    this.#transitions = new DocumentViewTransitions(document, ports.features);
    this.#visible = ports.observers.intersection((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting || !(entry.target instanceof window.HTMLAnchorElement)) continue;
        this.#requestPrefetch(entry.target);
        this.#visible?.unobserve(entry.target);
      }
    });
  }

  start(): void {
    if (this.#state === "disposed") throw new Error("navigation_runtime_disposed");
    if (this.#state === "running") return;
    this.#state = "running";
    this.#listen(true);
    this.#scan(this.#document.querySelectorAll("a"));
    this.#transitions.start();
    if (!this.#focusApplied) {
      this.#focusApplied = true;
      applyDocumentFocusScroll(this.#document);
    }
  }

  suspend(): void {
    if (this.#state !== "running") return;
    this.#listen(false);
    this.#visible?.disconnect();
    this.#prefetch.cancelAll();
    this.#state = "suspended";
  }

  resume(): void {
    if (this.#state !== "suspended") return;
    this.start();
  }

  mutations(records: readonly MutationRecord[]): void {
    if (this.#state !== "running") return;
    for (const record of records) {
      for (const removed of record.removedNodes) {
        for (const anchor of removedAnchors(this.#document, removed)) this.#cancelPrefetch(anchor);
      }
      for (const added of record.addedNodes) {
        const anchors = removedAnchors(this.#document, added);
        this.#scan(anchors);
      }
    }
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#listen(false);
    this.#visible?.disconnect();
    this.#prefetch.dispose();
    this.#transitions.cancel();
    this.#state = "disposed";
  }

  readonly #click = (event: MouseEvent): void => {
    if (event.defaultPrevented || !event.isTrusted) return;
    const anchor = anchorFromEvent(this.#document, event);
    if (anchor === null) return;
    const contract = anchorContract(anchor, event);
    if (contract === null || !currentDocumentDeparts(contract, anchor)) return;
    if (this.#guard.attempt(anchor) === "stay") {
      event.preventDefault();
      this.#transitions.cancel();
      return;
    }
    this.#prefetch.cancelAll();
    this.#transitions.prepare(contract.intent);
  };

  readonly #submit = (event: SubmitEvent): void => {
    if (event.defaultPrevented || !event.isTrusted) return;
    const Form = this.#document.defaultView?.HTMLFormElement;
    const HTMLElementConstructor = this.#document.defaultView?.HTMLElement;
    if (
      Form === undefined ||
      HTMLElementConstructor === undefined ||
      !(event.target instanceof Form)
    ) {
      return;
    }
    const submitter = event.submitter instanceof HTMLElementConstructor ? event.submitter : null;
    const intent = formIntent(event.target, submitter);
    if (intent === null) return;
    if (!formDepartsCurrentDocument(event.target, submitter)) return;
    if (this.#guard.attempt(submitter ?? event.target) === "stay") {
      event.preventDefault();
      return;
    }
    this.#prefetch.cancelAll();
  };

  readonly #beforeUnload = (event: BeforeUnloadEvent): void => {
    this.#guard.beforeUnload(event);
  };

  readonly #prefetchStart = (event: Event): void => {
    const anchor = anchorFromEvent(this.#document, event);
    if (anchor !== null) this.#requestPrefetch(anchor);
  };

  readonly #prefetchEnd = (event: Event): void => {
    const anchor = anchorFromEvent(this.#document, event);
    if (anchor === null) return;
    const related: unknown = Reflect.get(event, "relatedTarget");
    if (related instanceof Node && anchor.contains(related)) return;
    this.#cancelPrefetch(anchor);
  };

  #listen(add: boolean): void {
    const click = this.#click as EventListener;
    const submit = this.#submit as EventListener;
    if (add) {
      this.#document.addEventListener("click", click);
      this.#document.addEventListener("submit", submit);
      this.#document.addEventListener("pointerover", this.#prefetchStart);
      this.#document.addEventListener("focusin", this.#prefetchStart);
      this.#document.addEventListener("pointerout", this.#prefetchEnd);
      this.#document.addEventListener("focusout", this.#prefetchEnd);
      this.#document.defaultView?.addEventListener("beforeunload", this.#beforeUnload);
    } else {
      this.#document.removeEventListener("click", click);
      this.#document.removeEventListener("submit", submit);
      this.#document.removeEventListener("pointerover", this.#prefetchStart);
      this.#document.removeEventListener("focusin", this.#prefetchStart);
      this.#document.removeEventListener("pointerout", this.#prefetchEnd);
      this.#document.removeEventListener("focusout", this.#prefetchEnd);
      this.#document.defaultView?.removeEventListener("beforeunload", this.#beforeUnload);
    }
  }

  #scan(candidates: Iterable<Element>): void {
    let count = 0;
    const Anchor = this.#document.defaultView?.HTMLAnchorElement;
    if (Anchor === undefined) return;
    for (const candidate of candidates) {
      if (count >= NAVIGATION_RUNTIME_LIMITS.maxPrefetchTargets) break;
      if (!(candidate instanceof Anchor)) continue;
      count += 1;
      const contract = anchorContract(candidate);
      if (contract === null || contract.intent.prefetch === "none") continue;
      const declaration = contract.prefetch ?? contract.navigate;
      if (declaration?.modifiers.includes("eager") === true) {
        this.#requestPrefetch(candidate);
      } else if (declaration?.modifiers.includes("visible") === true) {
        this.#visible?.observe(candidate);
      }
    }
  }

  #key(anchor: HTMLAnchorElement): string {
    let key = this.#keys.get(anchor);
    if (key !== undefined) return key;
    this.#nextKey += 1;
    key = `prefetch-${String(this.#nextKey)}`;
    this.#keys.set(anchor, key);
    return key;
  }

  #requestPrefetch(anchor: HTMLAnchorElement): void {
    if (!anchor.isConnected) return;
    const contract = anchorContract(anchor);
    if (contract === null || contract.intent.prefetch === "none") return;
    this.#prefetch.request(this.#key(anchor), contract.intent, prefetchContext(anchor));
  }

  #cancelPrefetch(anchor: HTMLAnchorElement): void {
    const key = this.#keys.get(anchor);
    if (key !== undefined) this.#prefetch.cancel(key);
  }
}
