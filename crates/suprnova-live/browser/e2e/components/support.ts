import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Page } from "@playwright/test";

// The component cases drive each shipped custom element in a real engine
// with no server: the page holds the markup a component's view renders, the
// base stylesheet, and the component's own vendored stylesheet and script,
// the way an application page includes them.
const here = dirname(fileURLToPath(import.meta.url));
const componentsRoot = join(here, "..", "..", "..", "components");
const baseStylesheet = join(here, "..", "..", "src", "styles", "suprnova-ui.css");

export interface ComponentDocument {
  /** The body markup. */
  readonly html: string;
  /** The component directories whose stylesheet and script the page loads. */
  readonly components: readonly string[];
  /** Pins the document's color scheme, as `data-theme` on the root. */
  readonly theme?: "light" | "dark";
  /**
   * Counts the event listeners and mutation observers the component scripts
   * hold, read in the page with `__snBindings()`.
   */
  readonly instrument?: boolean;
}

// Installed before any component script, so every listener and observer a
// connection creates is counted, and every one it releases (removal, an
// aborted signal, a disconnect) is uncounted.
const INSTRUMENTATION = `(() => {
  const listeners = new Set();
  const add = EventTarget.prototype.addEventListener;
  const remove = EventTarget.prototype.removeEventListener;
  const captureOf = (options) => typeof options === "boolean" ? options : Boolean(options && options.capture);
  const find = (target, type, listener, capture) => {
    for (const entry of listeners) {
      if (entry.target === target && entry.type === type && entry.listener === listener && entry.capture === capture) return entry;
    }
    return null;
  };
  EventTarget.prototype.addEventListener = function (type, listener, options) {
    add.call(this, type, listener, options);
    if (listener === null || listener === undefined || this instanceof AbortSignal) return;
    const capture = captureOf(options);
    const signal = options !== null && typeof options === "object" ? options.signal : undefined;
    if ((signal !== undefined && signal.aborted) || find(this, type, listener, capture) !== null) return;
    const entry = { target: this, type, listener, capture };
    listeners.add(entry);
    if (signal !== undefined) add.call(signal, "abort", () => listeners.delete(entry), { once: true });
  };
  EventTarget.prototype.removeEventListener = function (type, listener, options) {
    remove.call(this, type, listener, options);
    const entry = find(this, type, listener, captureOf(options));
    if (entry !== null) listeners.delete(entry);
  };
  const observers = new Set();
  const observe = MutationObserver.prototype.observe;
  const disconnect = MutationObserver.prototype.disconnect;
  MutationObserver.prototype.observe = function (target, options) {
    observe.call(this, target, options);
    observers.add(this);
  };
  MutationObserver.prototype.disconnect = function () {
    disconnect.call(this);
    observers.delete(this);
  };
  globalThis.__snBindings = () => ({ listeners: listeners.size, observers: observers.size });
})();`;

export async function mountComponents(page: Page, document: ComponentDocument): Promise<void> {
  const theme = document.theme === undefined ? "" : ` data-theme="${document.theme}"`;
  await page.setContent(
    `<!doctype html><html lang="en"${theme}><head><meta charset="utf-8"><title>Component case</title></head><body>${document.html}</body></html>`,
  );
  await page.addStyleTag({ path: baseStylesheet });
  if (document.instrument === true) await page.addScriptTag({ content: INSTRUMENTATION });
  for (const component of document.components) {
    const stylesheet = join(componentsRoot, component, `${component}.css`);
    if (existsSync(stylesheet)) await page.addStyleTag({ path: stylesheet });
    const script = join(componentsRoot, component, `${component}.js`);
    if (existsSync(script)) await page.addScriptTag({ path: script });
  }
}

/**
 * Settles one step within a bound, so a renderer that stops running script
 * fails the case with a reason instead of hanging it.
 */
export async function within<T>(step: Promise<T>, milliseconds: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`${what} did not finish within ${String(milliseconds)} ms`));
    }, milliseconds);
  });
  try {
    return await Promise.race([step, expired]);
  } finally {
    clearTimeout(timer);
  }
}

/** Resolves once the page runs a script, within three seconds. */
export async function expectResponsive(page: Page): Promise<void> {
  await within(
    page.evaluate(() => true),
    3_000,
    "a script in the page",
  );
}
