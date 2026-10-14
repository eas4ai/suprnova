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

// NAV-006: a load-more append keeps every row already in the list. The
// library keys each row with a stable data-suprnova-live-key, so the morph
// plan pairs every existing row with its replacement, inserts only the new
// keys, and removes nothing; on the last page the server renders no control
// and the list keeps its rows all the same.

function row(document: FakeDocument, key: string): FakeElement {
  return element(document, "li", { class: "sn-feed-item", "data-suprnova-live-key": key }, [
    text(document, key),
  ]);
}

function feed(document: FakeDocument, keys: readonly string[], exhausted: boolean): FakeElement[] {
  const list = element(
    document,
    "ul",
    { class: "sn-load-more-list", id: "feed" },
    keys.map((key) => row(document, key)),
  );
  if (exhausted) return [list];
  return [
    list,
    element(
      document,
      "button",
      { class: "sn-load-more", type: "button", "live:click": "load_more" },
      [text(document, "Load more")],
    ),
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
  return { fixture, plan };
}

const FIRST_PAGE = ["row-1", "row-2", "row-3"];
const SECOND_PAGE = [...FIRST_PAGE, "row-4", "row-5", "row-6"];

describe("a keyed list across a load-more append", () => {
  it("pairs every existing row with its replacement and inserts only the new keys", () => {
    const { plan } = planFor(
      feed(new FakeDocument(), FIRST_PAGE, false),
      feed(new FakeDocument(), SECOND_PAGE, false),
    );
    for (const key of FIRST_PAGE) {
      const entry = plan.identity.entries.find((candidate) => candidate.value === key);
      if (entry === undefined) throw new Error(`row ${key} has no identity entry`);
      expect(entry.kind).toBe("live_key");
      expect(entry.current).not.toBeNull();
      expect(entry.replacement).not.toBeNull();
    }
    expect(plan.identity.inserted).toEqual(["row-4", "row-5", "row-6"]);
    expect(plan.identity.removed).toEqual([]);
    expect(plan.identity.moved).toEqual([]);
  });

  it("keeps the rows and drops nothing keyed when the server renders the last page without a control", () => {
    const { fixture, plan } = planFor(
      feed(new FakeDocument(), SECOND_PAGE, false),
      feed(new FakeDocument(), [...SECOND_PAGE, "row-7", "row-8", "row-9"], true),
    );
    expect(plan.identity.removed).toEqual([]);
    expect(plan.identity.inserted).toEqual(["row-7", "row-8", "row-9"]);
    const controls = fixture.replacementRoot.children.filter(
      (child) => child.getAttribute("class") === "sn-load-more",
    );
    expect(controls).toEqual([]);
  });

  it("reports a row the server dropped, so an unkeyed rewrite cannot pass as an append", () => {
    const { plan } = planFor(
      feed(new FakeDocument(), FIRST_PAGE, false),
      feed(new FakeDocument(), ["row-2", "row-3", "row-4"], false),
    );
    expect(plan.identity.removed).toEqual(["row-1"]);
    expect(plan.identity.inserted).toEqual(["row-4"]);
  });
});
