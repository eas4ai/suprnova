import { describe, expect, it } from "vitest";

import { preflightIslandMorph } from "../src/morph/preflight.js";
import type { MorphPlan } from "../src/morph/types.js";
import {
  asElement,
  element,
  FakeDocument,
  type FakeElement,
  morphFixture,
  text,
} from "./support/morph-dom.js";

// DATA-003: the library's collections carry stable domain keys, so a
// reorder, an insert and a removal pair every surviving item with its
// replacement. The list group keys each li and the datatable keys each tr
// with data-suprnova-live-key, the attribute the morph reads.

function listItem(document: FakeDocument, key: string): FakeElement {
  return element(document, "li", { class: "sn-list-item", "data-suprnova-live-key": key }, [
    text(document, key),
  ]);
}

function listGroup(document: FakeDocument, keys: readonly string[]): FakeElement[] {
  return [
    element(
      document,
      "ul",
      { class: "sn-list-group", id: "activity", "aria-label": "Recent activity" },
      keys.map((key) => listItem(document, key)),
    ),
  ];
}

function tableRow(document: FakeDocument, key: string): FakeElement {
  return element(document, "tr", { class: "sn-datatable-row", "data-suprnova-live-key": key }, [
    element(document, "th", { scope: "row" }, [text(document, key)]),
  ]);
}

function table(document: FakeDocument, keys: readonly string[]): FakeElement[] {
  return [
    element(document, "table", { class: "sn-datatable", id: "invoices" }, [
      element(
        document,
        "tbody",
        {},
        keys.map((key) => tableRow(document, key)),
      ),
    ]),
  ];
}

function planFor(current: FakeElement[], replacement: FakeElement[]) {
  const fixture = morphFixture({ currentChildren: current, replacementChildren: replacement });
  const plan: MorphPlan = preflightIslandMorph({
    authority: fixture.authority,
    currentRoot: asElement(fixture.currentRoot),
    html: "<section></section>",
    limits: fixture.limits,
    parser: fixture.parser,
  });
  return plan;
}

const ITEMS = ["act-1", "act-2", "act-3", "act-4"];

describe("a keyed list group across a reorder", () => {
  it("pairs every item with its replacement and moves instead of replacing", () => {
    const plan = planFor(
      listGroup(new FakeDocument(), ITEMS),
      listGroup(new FakeDocument(), [...ITEMS].reverse()),
    );
    for (const key of ITEMS) {
      const entry = plan.identity.entries.find((candidate) => candidate.value === key);
      if (entry === undefined) throw new Error(`item ${key} has no identity entry`);
      expect(entry.kind).toBe("live_key");
      expect(entry.current).not.toBeNull();
      expect(entry.replacement).not.toBeNull();
    }
    expect(plan.identity.inserted).toEqual([]);
    expect(plan.identity.removed).toEqual([]);
    expect(plan.identity.moved.length).toBeGreaterThan(0);
  });

  it("reports exactly the item the server dropped and the one it added", () => {
    const plan = planFor(
      listGroup(new FakeDocument(), ITEMS),
      listGroup(new FakeDocument(), ["act-5", "act-1", "act-2", "act-4"]),
    );
    expect(plan.identity.removed).toEqual(["act-3"]);
    expect(plan.identity.inserted).toEqual(["act-5"]);
  });
});

describe("keyed datatable rows across a sort", () => {
  it("keeps every row that survives the sort and pairs it by key", () => {
    const keys = ["inv-1", "inv-2", "inv-3", "inv-4"];
    const plan = planFor(
      table(new FakeDocument(), keys),
      table(new FakeDocument(), ["inv-4", "inv-2", "inv-1", "inv-3"]),
    );
    expect(plan.identity.inserted).toEqual([]);
    expect(plan.identity.removed).toEqual([]);
    expect(plan.identity.entries.filter((entry) => entry.kind === "live_key")).toHaveLength(4);
  });

  it("reports the rows a filter removed and the rows a page change brought in", () => {
    const plan = planFor(
      table(new FakeDocument(), ["inv-1", "inv-2", "inv-3", "inv-4"]),
      table(new FakeDocument(), ["inv-2", "inv-6"]),
    );
    expect(plan.identity.removed).toEqual(["inv-1", "inv-3", "inv-4"]);
    expect(plan.identity.inserted).toEqual(["inv-6"]);
  });
});
