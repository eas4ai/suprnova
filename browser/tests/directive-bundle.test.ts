import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const BROWSER_ROOT = fileURLToPath(new URL("../", import.meta.url));
const CAPABILITY_MARKERS = ["uploads@1", "async@1"] as const;
const OPTIONAL_TABLE_MARKERS = ["push-only", "hybrid", "15s", "30s", "60s"] as const;

function assertCoreOutputHasNoOptionalContracts(output: string): void {
  for (const marker of [...CAPABILITY_MARKERS, ...OPTIONAL_TABLE_MARKERS]) {
    if (output.includes(marker)) throw new TypeError(`optional_contract_data:${marker}`);
  }
}

describe("core directive production boundary", () => {
  it("excludes the optional parser, capability markers, and feature tables from both core entries", async () => {
    for (const [entryPoint, format] of [
      ["src/entry-esm.ts", "esm"],
      ["src/entry-classic.ts", "iife"],
    ] as const) {
      const result = await build({
        absWorkingDir: BROWSER_ROOT,
        bundle: true,
        entryPoints: [entryPoint],
        format,
        metafile: true,
        minify: true,
        platform: "browser",
        treeShaking: true,
        write: false,
      });
      const inputs = Object.keys(result.metafile.inputs).map((name) => name.split("\\").join("/"));
      expect(inputs.some((name) => name.endsWith("/features/directive-parser.ts"))).toBe(false);
      const output = result.outputFiles[0];
      expect(output).toBeDefined();
      if (output === undefined) throw new TypeError("core_bundle_output_missing");
      expect(() => {
        assertCoreOutputHasNoOptionalContracts(output.text);
      }).not.toThrow();
    }
  });

  it("detects representative optional contract contamination independent of formatting", () => {
    for (const marker of [...CAPABILITY_MARKERS, ...OPTIONAL_TABLE_MARKERS]) {
      expect(() => {
        assertCoreOutputHasNoOptionalContracts(
          `const harmlessFormatting = ${JSON.stringify(marker)};`,
        );
      }).toThrow(`optional_contract_data:${marker}`);
    }
  });
});
