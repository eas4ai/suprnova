// FDB-005: the live feed and the notification bell render a degraded stream
// state honestly. The runtime owns the status: it writes
// data-live-stream-state on the island root and announces every change into
// the [data-live-stream-status] element the shipped views render. This
// fixture drives the runtime's feedback layer over the feed's markup shape
// and checks the shipped views and stylesheets against the runtime's
// vocabulary, so a retired or reconnecting stream never reads as current.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { FeedbackRuntime } from "../src/feedback/targets.js";
import type { IslandMetadata } from "../src/islands/metadata.js";
import { IslandRecord } from "../src/islands/record.js";
import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import type { SubscriptionState } from "../src/async-updates/types.js";

const COMPONENTS = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "components");

class FakeElement {
  readonly #attributes = new Map<string, string>();
  readonly children: FakeElement[] = [];
  parent: FakeElement | null = null;
  isConnected = true;
  textContent = "";

  constructor(attributes: Readonly<Record<string, string>> = {}) {
    for (const [name, value] of Object.entries(attributes)) this.#attributes.set(name, value);
  }

  append(child: FakeElement): FakeElement {
    child.parent = this;
    this.children.push(child);
    return child;
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.#attributes.has(name);
  }

  removeAttribute(name: string): void {
    this.#attributes.delete(name);
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }

  // The runtime's selectors here are attribute presence selectors.
  matches(selector: string): boolean {
    const name = /^\[([a-z-]+)\]$/u.exec(selector)?.[1];
    return name !== undefined && this.#attributes.has(name);
  }

  closest(selector: string): FakeElement | null {
    if (this.matches(selector)) return this;
    return this.parent?.closest(selector) ?? null;
  }

  contains(other: FakeElement): boolean {
    let current: FakeElement | null = other;
    while (current !== null) {
      if (current === this) return true;
      current = current.parent;
    }
    return false;
  }

  querySelectorAll(selector: string): FakeElement[] {
    const found: FakeElement[] = [];
    for (const child of this.children) {
      if (child.matches(selector)) found.push(child);
      found.push(...child.querySelectorAll(selector));
    }
    return found;
  }

  querySelector(selector: string): FakeElement | null {
    return this.querySelectorAll(selector)[0] ?? null;
  }
}

const immediate: RuntimeScheduler = {
  animationFrame: () => 1,
  cancelAnimationFrame: () => {
    // Nothing is scheduled on frames here.
  },
  clearTimeout: () => {
    // Nothing is scheduled on timers here.
  },
  microtask: (callback) => {
    callback();
  },
  timeout: () => 1,
};
const clock: RuntimeClock = { now: () => 0 };

function feedIsland(): { island: FakeElement; status: FakeElement; record: IslandRecord } {
  const island = new FakeElement({ "data-suprnova-live-island": "", "aria-busy": "false" });
  const section = island.append(new FakeElement({ class: "sn-live-feed" }));
  const status = section.append(
    new FakeElement({ "data-live-stream-status": "", role: "status", "aria-live": "polite" }),
  );
  status.textContent = "Updates disconnected";
  const metadata = {
    component: "app.live-native-gallery",
    documentKey: "live-native-gallery",
    instanceId: null,
    lazyComplete: false,
    protocolMinimum: 2,
    revision: 1n,
    runtimeContract: 1,
    slot: "gallery",
    snapshot: {},
    snapshotForm: "instanced",
  } as unknown as IslandMetadata;
  const record = new IslandRecord(island as unknown as Element, metadata);
  return { island, record, status };
}

describe("live feed stream status", () => {
  it("names degraded, reconnecting and closed streams as such, and only the current state as current", () => {
    const { island, record, status } = feedIsland();
    const runtime = new FeedbackRuntime(clock, immediate);
    runtime.connect(record, [], null);
    const seen = new Map<
      SubscriptionState,
      { state: string | null; busy: string | null; text: string }
    >();
    for (const state of ["connecting", "current", "degraded", "reconnecting", "closed"] as const) {
      runtime.setAsyncStatus(record, state);
      seen.set(state, {
        busy: island.getAttribute("aria-busy"),
        state: island.getAttribute("data-live-stream-state"),
        text: status.textContent,
      });
    }
    expect(seen.get("current")).toEqual({
      busy: "false",
      state: "current",
      text: "Updates current",
    });
    expect(seen.get("degraded")).toEqual({
      busy: "false",
      state: "degraded",
      text: "Updates degraded",
    });
    expect(seen.get("reconnecting")).toEqual({
      busy: "true",
      state: "reconnecting",
      text: "Reconnecting to updates",
    });
    expect(seen.get("closed")).toEqual({ busy: "false", state: "closed", text: "Updates closed" });
    for (const [state, { text }] of seen) {
      if (state === "current") continue;
      expect(text.toLowerCase()).not.toContain("current");
      expect(text.toLowerCase()).not.toContain("live");
    }
    expect(status.getAttribute("role")).toBe("status");
    expect(status.getAttribute("aria-live")).toBe("polite");
    runtime.retire(record);
    expect(island.getAttribute("data-live-stream-state")).toBeNull();
  });

  it("ships the disconnected message as the server default and colors the degraded states", () => {
    for (const [component, statusClass] of [
      ["live-feed", ".sn-live-feed-status"],
      ["notification-bell", ".sn-bell-status"],
    ] as const) {
      const html = readFileSync(join(COMPONENTS, component, `${component}.html`), "utf8");
      const css = readFileSync(join(COMPONENTS, component, `${component}.css`), "utf8");
      const statusTag = /<[a-z]+[^>]*data-live-stream-status[^>]*>([^<]*)</u.exec(html);
      expect(statusTag?.[1]).toBe("Updates disconnected");
      expect(statusTag?.[0]).toContain('role="status"');
      expect(statusTag?.[0]).toContain('aria-live="polite"');
      for (const state of ["degraded", "reconnecting", "closed"]) {
        expect(css).toContain(`[data-live-stream-state="${state}"] ${statusClass}`);
      }
      expect(css).toContain(`[data-live-stream-state="current"] ${statusClass}`);
      const text = statusTag?.[1]?.toLowerCase() ?? "";
      expect(text).not.toContain("live");
      expect(text).not.toContain("current");
    }
  });
});
