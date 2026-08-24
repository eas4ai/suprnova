import { build } from "esbuild";
import { describe, expect, it } from "vitest";

const browserRoot = new URL("../", import.meta.url).pathname;

async function inputs(source: string): Promise<readonly string[]> {
  const result = await build({
    absWorkingDir: browserRoot,
    bundle: true,
    format: "esm",
    metafile: true,
    platform: "browser",
    stdin: {
      contents: source,
      resolveDir: browserRoot,
      sourcefile: "feature-graph-entry.ts",
    },
    target: ["chrome111", "edge111", "firefox128", "safari16.4"],
    treeShaking: true,
    write: false,
  });
  return Object.keys(result.metafile.inputs).map((name) => name.replace(/\\/gu, "/"));
}

function stimulusImplementation(names: readonly string[]): readonly string[] {
  return names.filter((name) =>
    /(?:stimulus\/bridge\.ts|stimulus\/lifecycle\.ts|@hotwired\/stimulus)/u.test(name),
  );
}

describe("optional feature import boundaries", () => {
  it("keeps core, uploads, async, contract, and producer graphs free of Stimulus behavior", async () => {
    const names = await inputs(`
      import { boot } from "./src/bootstrap.ts";
      import { defineAsyncFeature, defineUploadsFeature } from "./src/features/contract.ts";
      import { registerRuntimeFeature } from "./src/features/producer.ts";
      export { boot, defineAsyncFeature, defineUploadsFeature, registerRuntimeFeature };
    `);

    expect(stimulusImplementation(names)).toEqual([]);
  });

  it("retains Stimulus behavior only in the Stimulus adapter graph", async () => {
    const names = await inputs(`
      export { installStimulusAdapter } from "./src/features/stimulus.ts";
    `);

    expect(names.some((name) => name.endsWith("stimulus/bridge.ts"))).toBe(true);
    expect(names.some((name) => name.endsWith("stimulus/lifecycle.ts"))).toBe(true);
    expect(names.some((name) => name.includes("@hotwired/stimulus"))).toBe(false);
  });
});
