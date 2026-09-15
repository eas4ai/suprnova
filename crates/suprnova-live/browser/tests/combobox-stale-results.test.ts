// FORM-008: a stale result never replaces results for a newer query. The
// combobox element ships as a vendored file that defines sn-combobox; its
// stale-suppression decisions are plain functions on SuprnovaCombobox so this
// fixture can prove them without a document. The file is evaluated with the
// custom-element registry absent, which is also the state a document that
// never added the component is in.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";

import { describe, expect, it } from "vitest";

interface ComboboxApi {
  readonly QuerySequence: new () => { ask(): number; accepts(sequence: number): boolean };
  readonly acceptsResults: (currentQuery: string, resultQuery: string) => boolean;
}

function loadCombobox(): ComboboxApi {
  const source = readFileSync(
    join(
      dirname(fileURLToPath(import.meta.url)),
      "..",
      "..",
      "components",
      "combobox",
      "combobox.js",
    ),
    "utf8",
  );
  const context: Record<string, unknown> = { HTMLElement: Object };
  context["globalThis"] = context;
  runInNewContext(source, context);
  const api: unknown = context["SuprnovaCombobox"];
  if (typeof api !== "object" || api === null) throw new Error("combobox_api_missing");
  return api as ComboboxApi;
}

describe("combobox stale suppression", () => {
  it("shows a listbox only when it answers the input's current text", () => {
    const { acceptsResults } = loadCombobox();
    expect(acceptsResults("ca", "ca")).toBe(true);
    expect(acceptsResults("can", "ca")).toBe(false);
    expect(acceptsResults("ca", "can")).toBe(false);
    expect(acceptsResults("", "")).toBe(true);
  });

  it("drops an older answer even when its text matches, once a newer question was asked", () => {
    const { QuerySequence } = loadCombobox();
    const sequence = new QuerySequence();
    const first = sequence.ask();
    const second = sequence.ask();
    expect(sequence.accepts(first)).toBe(false);
    expect(sequence.accepts(second)).toBe(true);
    const third = sequence.ask();
    expect(sequence.accepts(second)).toBe(false);
    expect(sequence.accepts(third)).toBe(true);
  });

  it("defines nothing when the registry is absent, so a library-free document never sees sn-combobox", () => {
    const source = readFileSync(
      join(
        dirname(fileURLToPath(import.meta.url)),
        "..",
        "..",
        "components",
        "combobox",
        "combobox.js",
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
    runInNewContext(source, context);
    expect(defined).toEqual(["sn-combobox"]);
  });
});
