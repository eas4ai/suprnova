import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import vm from "node:vm";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

interface AssetEntry {
  readonly file: string;
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: string;
  readonly content_type: string;
  readonly script_kind: "module" | "classic";
  readonly preload_rel: "modulepreload" | "preload";
  readonly cache_control: string;
}

interface AssetManifest {
  readonly schema_version: 1;
  readonly engine_version: string;
  readonly runtime_contract_version: 1;
  readonly protocol_versions: readonly number[];
  readonly snapshot_versions: readonly number[];
  readonly built_at: string;
  readonly assets: readonly AssetEntry[];
  readonly provenance: {
    readonly idiomorph: {
      readonly name: "idiomorph";
      readonly version: "0.7.4";
      readonly license: "0BSD";
      readonly bundled: boolean;
    };
  };
}

const browserRoot = new URL("../", import.meta.url);
const buildScript = new URL("scripts/build.mjs", browserRoot);
let temporaryRoot = "";
let outputDirectory = "";

function runBuild(outdir: string): void {
  execFileSync(process.execPath, [buildScript.pathname, "--outdir", outdir], {
    cwd: new URL("..", browserRoot),
    stdio: "pipe",
  });
}

async function bytes(name: string): Promise<Buffer> {
  return readFile(join(outputDirectory, name));
}

beforeAll(async () => {
  temporaryRoot = await mkdtemp(join(tmpdir(), "suprnova-live-build-contract-"));
  outputDirectory = join(temporaryRoot, "first");
  runBuild(outputDirectory);
});

afterAll(async () => {
  if (temporaryRoot.length > 0) await rm(temporaryRoot, { recursive: true, force: true });
});

describe("deterministic production assets", () => {
  it("emits only the exact ESM, classic, declaration, and manifest files", async () => {
    expect((await readdir(outputDirectory)).sort()).toEqual([
      "index.d.ts",
      "suprnova-live.assets.json",
      "suprnova-live.classic.js",
      "suprnova-live.esm.js",
    ]);
  });

  it("publishes the host-port and typed asset-manifest contracts", async () => {
    const declarations = await readFile(join(outputDirectory, "index.d.ts"), "utf8");
    expect(declarations).toContain("export interface RuntimePortOverrides");
    expect(declarations).toContain(
      "export interface BootstrapOptions extends RuntimePortOverrides",
    );
    expect(declarations).toContain("export interface RuntimeAssetManifest");
    expect(declarations).toContain("export interface EffectRegistration");
    expect(declarations).toContain("export interface RuntimeCallRegistration");
    expect(declarations).toContain("export interface StimulusApplicationPort");
    expect(declarations).toContain("export interface StimulusMorphBridge");
    expect(declarations).toContain("beforeMorph(scope: Element): StimulusContinuity");
    expect(declarations).toContain("unload(...identifiers: readonly string[]): void");
    expect(declarations).toContain("readonly stimulus?: StimulusBootstrapOptions");
    expect(declarations).toContain("runEffect(owner: Element, invocation: EffectInvocation)");
  });

  it("records exact versions, hashes, serving intent, cache policy, and bundled provenance", async () => {
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "suprnova-live.assets.json"), "utf8"),
    ) as AssetManifest;

    expect(manifest).toMatchObject({
      schema_version: 1,
      engine_version: "0.1.0",
      runtime_contract_version: 1,
      protocol_versions: [1, 2],
      snapshot_versions: [1],
      built_at: "1970-01-01T00:00:00.000Z",
      provenance: {
        idiomorph: {
          name: "idiomorph",
          version: "0.7.4",
          license: "0BSD",
          bundled: true,
        },
      },
    });
    expect(manifest.assets.map(({ file }) => file)).toEqual([
      "suprnova-live.classic.js",
      "suprnova-live.esm.js",
    ]);
    for (const asset of manifest.assets) {
      const content = await bytes(asset.file);
      const digest = createHash("sha256").update(content).digest();
      expect(asset.bytes).toBe(content.byteLength);
      expect(asset.sha256).toBe(digest.toString("hex"));
      expect(asset.sri).toBe(`sha256-${digest.toString("base64")}`);
      expect(asset.content_type).toBe("text/javascript; charset=utf-8");
      expect(asset.cache_control).toBe("public, max-age=31536000, immutable");
      expect(asset.preload_rel).toBe(asset.script_kind === "module" ? "modulepreload" : "preload");
    }
  });

  it("exposes equivalent ESM and non-replaceable classic facades from one singleton core", async () => {
    const esmSource = await bytes("suprnova-live.esm.js");
    const esm = (await import(`data:text/javascript;base64,${esmSource.toString("base64")}`)) as {
      readonly version: string;
      readonly runtimeContractVersion: number;
      readonly supportedProtocolVersions: readonly number[];
      readonly RUNTIME_SYMBOL: symbol;
      readonly boot: unknown;
    };
    const classicWindow: Record<string, unknown> = {};
    vm.runInNewContext(await readFile(join(outputDirectory, "suprnova-live.classic.js"), "utf8"), {
      window: classicWindow,
    });
    const classic = classicWindow["SuprnovaLive"] as typeof esm;
    const descriptor = Object.getOwnPropertyDescriptor(classicWindow, "SuprnovaLive");

    expect(esm.version).toBe("0.1.0");
    expect(Symbol.keyFor(esm.RUNTIME_SYMBOL)).toBe("suprnova.live.runtime.v1");
    expect(classic.version).toBe(esm.version);
    expect(classic.runtimeContractVersion).toBe(esm.runtimeContractVersion);
    expect(classic.supportedProtocolVersions).toEqual(esm.supportedProtocolVersions);
    expect(typeof esm.boot).toBe("function");
    expect(typeof classic.boot).toBe("function");
    expect(descriptor).toMatchObject({ configurable: false, writable: false });
  });

  it("contains no production maps or unsafe/dynamic evaluation forms", async () => {
    for (const name of ["suprnova-live.classic.js", "suprnova-live.esm.js"]) {
      const source = (await bytes(name)).toString("utf8");
      expect(source).toContain("Idiomorph 0.7.4");
      expect(source).not.toContain("sourceMappingURL");
      expect(source).not.toMatch(/\beval\s*\(/u);
      expect(source).not.toMatch(/\bnew\s+Function\b/u);
      expect(source).not.toMatch(/\bimport\s*\(/u);
      expect(source).not.toContain("@hotwired/stimulus");
    }
  });

  it("rebuilds every production byte identically", async () => {
    const second = join(temporaryRoot, "second");
    runBuild(second);
    const names = (await readdir(outputDirectory)).sort();
    expect((await readdir(second)).sort()).toEqual(names);
    for (const name of names) {
      expect(await readFile(join(second, name))).toEqual(await bytes(name));
    }
  });
});
