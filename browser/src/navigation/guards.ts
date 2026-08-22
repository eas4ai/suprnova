export const NAVIGATION_GUARD_ATTRIBUTE = "data-suprnova-live-navigation-guard";
export const DIRTY_WORK_ATTRIBUTE = "data-suprnova-live-dirty";

const MAX_GUARDS = 32;
const MAX_MESSAGE_UNITS = 256;

export interface DirtyGuardPrompt {
  confirm(message: string): boolean;
  defer(callback: VoidFunction): void;
}

export type DirtyGuardDisposition = "leave" | "stay";

function checkedMessage(element: Element): string | null {
  const message = element.getAttribute(NAVIGATION_GUARD_ATTRIBUTE)?.trim() ?? "";
  if (message.length === 0 || message.length > MAX_MESSAGE_UNITS || hasControlCharacter(message)) {
    return null;
  }
  return message;
}

function hasControlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

export function declaredDirtyMessage(document: Document): string | null {
  const candidates = document.querySelectorAll(
    `[${NAVIGATION_GUARD_ATTRIBUTE}][${DIRTY_WORK_ATTRIBUTE}="true"]`,
  );
  if (candidates.length === 0 || candidates.length > MAX_GUARDS) return null;
  for (const candidate of candidates) {
    const message = checkedMessage(candidate);
    if (message !== null) return message;
  }
  return null;
}

function returnFocus(source: Element | null): void {
  if (source?.isConnected !== true) return;
  const window = source.ownerDocument.defaultView;
  if (window === null || !(source instanceof window.HTMLElement)) return;
  try {
    source.focus({ preventScroll: true });
  } catch {
    try {
      source.focus();
    } catch {
      // A failed focus return cannot turn stay into leave.
    }
  }
}

export class DirtyNavigationGuard {
  readonly #document: Document;
  readonly #prompt: DirtyGuardPrompt;
  #leaving = false;

  constructor(document: Document, prompt: DirtyGuardPrompt) {
    this.#document = document;
    this.#prompt = prompt;
  }

  attempt(source: Element | null, bypass = false): DirtyGuardDisposition {
    const message = bypass ? null : declaredDirtyMessage(this.#document);
    if (message === null) return "leave";
    if (!this.#prompt.confirm(message)) {
      returnFocus(source);
      return "stay";
    }
    this.#leaving = true;
    this.#prompt.defer(() => {
      this.#leaving = false;
    });
    return "leave";
  }

  beforeUnload(event: BeforeUnloadEvent): boolean {
    if (this.#leaving || declaredDirtyMessage(this.#document) === null) return false;
    event.preventDefault();
    Reflect.set(event, "returnValue", "");
    return true;
  }
}
