import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const BROWSER_ROOT = fileURLToPath(new URL("../", import.meta.url));

describe("core directive production boundary", () => {
  it("does not import the optional feature parser into the core entry", async () => {
    const result = await build({
      absWorkingDir: BROWSER_ROOT,
      bundle: true,
      entryPoints: ["src/entry-esm.ts"],
      format: "esm",
      metafile: true,
      platform: "browser",
      treeShaking: true,
      write: false,
    });
    const inputs = Object.keys(result.metafile.inputs).map((name) => name.split("\\").join("/"));
    expect(inputs.some((name) => name.endsWith("/features/directive-parser.ts"))).toBe(false);
  });
});
