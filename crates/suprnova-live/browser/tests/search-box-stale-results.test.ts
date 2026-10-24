// FORM-008: a stale result never replaces results for a newer query. The
// search-box element ships as a vendored file that defines sn-search-box; its
// stale-suppression decisions are plain functions in an object the file's
// function returns, which is the script's completion value, so this fixture
// can prove them without a document. The file writes nothing on the global
// object (REG-032). It is evaluated with the custom-element registry absent,
// which is also the state a document that never added the component is in.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

import { describe, expect, it } from "vitest";

interface SearchBoxApi {
  readonly QuerySequence: new () => { ask(): number; accepts(sequence: number): boolean };
  readonly acceptsResults: (currentQuery: string, resultQuery: string) => boolean;
}

function loadSearchBox(): SearchBoxApi {
  const source = readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "..",
      "..",
      "components",
      "search-box",
      "search-box.js",
    ),
    "utf8",
  );
  const context: Record<string, unknown> = { HTMLElement: Object };
  context["globalThis"] = context;
  const keys = Object.keys(context).sort();
  const api = searchBoxApi(runInNewContext(source, context));
  if (Object.keys(context).sort().join() !== keys.join()) {
    throw new Error("search-box_wrote_a_global");
  }
  return api;
}

// The decisions object a run of the file returned, or an error if it returned none.
function searchBoxApi(result: unknown): SearchBoxApi {
  if (typeof result !== "object" || result === null || !("acceptsResults" in result)) {
    throw new Error("search-box_api_missing");
  }
  return result as SearchBoxApi;
}

describe("search-box stale suppression", () => {
  it("shows a listbox only when it answers the input's current text", () => {
    const { acceptsResults } = loadSearchBox();
    expect(acceptsResults("ca", "ca")).toBe(true);
    expect(acceptsResults("can", "ca")).toBe(false);
    expect(acceptsResults("ca", "can")).toBe(false);
    expect(acceptsResults("", "")).toBe(true);
  });

  it("drops an older answer even when its text matches, once a newer question was asked", () => {
    const { QuerySequence } = loadSearchBox();
    const sequence = new QuerySequence();
    const first = sequence.ask();
    const second = sequence.ask();
    expect(sequence.accepts(first)).toBe(false);
    expect(sequence.accepts(second)).toBe(true);
    const third = sequence.ask();
    expect(sequence.accepts(second)).toBe(false);
    expect(sequence.accepts(third)).toBe(true);
  });

  it("defines nothing when the registry is absent, so a library-free document never sees sn-search-box", () => {
    const source = readFileSync(
      join(
        dirname(fileURLToPath(import.meta.url)),
        "..",
        "..",
        "components",
        "search-box",
        "search-box.js",
      ),
      "utf8",
    );
    const defined: string[] = [];
    const context: Record<string, unknown> = {
      HTMLElement: Object,
      customElements: {
        define: (name: string) => defined.push(name),
        get: () => undefined,
      },
    };
    context["globalThis"] = context;
    const keys = Object.keys(context).sort();
    const { acceptsResults } = searchBoxApi(runInNewContext(source, context));
    expect(defined).toEqual(["sn-search-box"]);
    expect(acceptsResults("ca", "ca")).toBe(true);
    expect(Object.keys(context).sort()).toEqual(keys);
  });
});
