export const DOCUMENT_FOCUS_ATTRIBUTE = "data-suprnova-live-document-focus";

export type FocusScrollDisposition = "focused" | "native";

function historyTraversal(window: Window): boolean {
  try {
    const entry = window.performance.getEntriesByType("navigation")[0] as
      PerformanceNavigationTiming | undefined;
    return entry?.type === "back_forward";
  } catch {
    return false;
  }
}

function fragmentTarget(document: Document, window: Window): HTMLElement | null {
  const platform = window as Window & typeof globalThis;
  if (window.location.hash.length <= 1) return null;
  let name: string;
  try {
    name = decodeURIComponent(window.location.hash.slice(1));
  } catch {
    return null;
  }
  const byId = document.getElementById(name);
  if (byId instanceof platform.HTMLElement) return byId;
  const named = document.getElementsByName(name);
  const candidate = named[0];
  return named.length === 1 && candidate instanceof platform.HTMLElement ? candidate : null;
}

function declaredTarget(document: Document, window: Window): HTMLElement | null {
  const platform = window as Window & typeof globalThis;
  const elements = document.querySelectorAll(`[${DOCUMENT_FOCUS_ATTRIBUTE}]`);
  const candidate = elements[0];
  return elements.length === 1 && candidate instanceof platform.HTMLElement ? candidate : null;
}

function userAlreadyFocused(document: Document): boolean {
  const active = document.activeElement;
  return active !== null && active !== document.body && active !== document.documentElement;
}

function focusTarget(target: HTMLElement, fragment: boolean): boolean {
  const hadTabIndex = target.hasAttribute("tabindex");
  if (!hadTabIndex) target.setAttribute("tabindex", "-1");
  try {
    target.focus({ preventScroll: fragment });
    if (fragment) target.scrollIntoView({ block: "start" });
    if (!hadTabIndex) {
      target.addEventListener(
        "blur",
        () => {
          target.removeAttribute("tabindex");
        },
        { once: true },
      );
    }
    return target.ownerDocument.activeElement === target;
  } catch {
    if (!hadTabIndex) target.removeAttribute("tabindex");
    return false;
  }
}

export function applyDocumentFocusScroll(document: Document): FocusScrollDisposition {
  const window = document.defaultView;
  if (window === null || historyTraversal(window) || userAlreadyFocused(document)) return "native";
  const fragment = fragmentTarget(document, window);
  const target = fragment ?? declaredTarget(document, window);
  if (target === null) return "native";
  return focusTarget(target, fragment !== null) ? "focused" : "native";
}
