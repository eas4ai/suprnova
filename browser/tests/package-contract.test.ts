import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";

interface PackageManifest {
  readonly name: string;
  readonly private: boolean;
  readonly types: string;
  readonly module: string;
  readonly sideEffects: readonly string[];
  readonly exports: Record<string, unknown>;
  readonly scripts: Record<string, string>;
  readonly dependencies?: Record<string, string>;
  readonly devDependencies: Record<string, string>;
}

const EXPECTED_SCRIPTS = [
  "budget",
  "budget:browser",
  "build",
  "build:check",
  "compatibility:check",
  "compatibility:run",
  "format",
  "format:check",
  "generate",
  "generate:check",
  "lint",
  "test",
  "test:browser",
  "test:browser:install",
  "test:unit",
  "typecheck",
] as const;

async function readPackageManifest(): Promise<PackageManifest> {
  const json = await readFile(new URL("../package.json", import.meta.url), "utf8");
  return JSON.parse(json) as PackageManifest;
}

describe("production browser package contract", () => {
  it("pins the runtime workspace identity, entry points, tools, and scripts", async () => {
    const manifest = await readPackageManifest();

    expect(manifest.name).toBe("@suprnova/live");
    expect(manifest.private).toBe(true);
    expect(manifest.types).toBe("./dist/index.d.ts");
    expect(manifest.module).toBe("./dist/suprnova-live.esm.js");
    expect(manifest.sideEffects).toEqual(["./dist/suprnova-live.classic.js"]);
    expect(manifest.exports).toEqual({
      ".": {
        types: "./dist/index.d.ts",
        import: "./dist/suprnova-live.esm.js",
      },
      "./runtime": {
        import: "./dist/suprnova-live.esm.js",
      },
    });
    expect(Object.keys(manifest.scripts).sort()).toEqual(EXPECTED_SCRIPTS);
    expect(manifest.scripts["test"]).toBe("npm run test:unit");
    expect(manifest.scripts["budget:browser"]).toBe("node scripts/run-browser-budget.mjs");

    expect(manifest.dependencies).toEqual({ idiomorph: "0.7.4" });
    expect(manifest.dependencies).not.toHaveProperty("@hotwired/stimulus");
    expect(manifest.devDependencies).toMatchObject({
      "@hotwired/stimulus": "3.2.2",
      "@playwright/test": "1.62.1",
      "axe-core": "4.13.0",
      esbuild: "0.28.2",
      "fast-check": "4.9.0",
    });
  });
});
