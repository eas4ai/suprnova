import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import vm from "node:vm";
import { brotliCompressSync, constants as zlibConstants } from "node:zlib";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  evaluateArtifactBudgets,
  validateArtifactSizeBaselineProvenance,
} from "../scripts/check-budget.mjs";
import { PRODUCTION_BUILD_HOOK_TIMEOUT_MS } from "./support/production-build.js";

interface AssetEntry {
  readonly file: string;
  readonly role: string;
  readonly bytes: number;
  readonly sha256: string;
  readonly sri: string;
  readonly content_type: string;
  readonly script_kind: "module" | "classic";
  readonly preload_rel: "modulepreload" | "preload";
  readonly cache_control: string;
  readonly capability: string;
  readonly capability_version: 1;
  readonly compatible_core: string;
}

interface AssetManifest {
  readonly schema_version: 2;
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
}, PRODUCTION_BUILD_HOOK_TIMEOUT_MS);

afterAll(async () => {
  if (temporaryRoot.length > 0) await rm(temporaryRoot, { recursive: true, force: true });
});

describe("deterministic production assets", () => {
  it("emits only the exact ESM, classic, declaration, and manifest files", async () => {
    expect((await readdir(outputDirectory)).sort()).toEqual([
      "index.d.ts",
      "suprnova-live.assets.json",
      "suprnova-live.async.classic.js",
      "suprnova-live.async.esm.js",
      "suprnova-live.classic.js",
      "suprnova-live.esm.js",
      "suprnova-live.stimulus.classic.js",
      "suprnova-live.stimulus.esm.js",
      "suprnova-live.uploads.classic.js",
      "suprnova-live.uploads.esm.js",
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
    expect(declarations).not.toContain("LifecycleTestProbe");
    expect(declarations).not.toContain("lifecycleTestProbe");
  });

  it("records exact versions, hashes, serving intent, cache policy, and bundled provenance", async () => {
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "suprnova-live.assets.json"), "utf8"),
    ) as AssetManifest;

    expect(manifest).toMatchObject({
      schema_version: 2,
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
      "suprnova-live.stimulus.classic.js",
      "suprnova-live.stimulus.esm.js",
      "suprnova-live.uploads.classic.js",
      "suprnova-live.uploads.esm.js",
      "suprnova-live.async.classic.js",
      "suprnova-live.async.esm.js",
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
      expect(asset.capability_version).toBe(1);
      expect(asset.compatible_core).toBe(">=0.1.0 <0.2.0");
    }
  });

  it("measures changed async candidates against immutable reviewed history", async () => {
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "suprnova-live.assets.json"), "utf8"),
    ) as AssetManifest;
    const baseline = JSON.parse(
      await readFile(
        new URL("../benchmarks/baselines/artifact-size-v1.json", import.meta.url),
        "utf8",
      ),
    ) as Readonly<{
      history: readonly Readonly<{
        review: Readonly<{
          decision: string;
          sourceCommit: string;
          sourceDecision?: string;
          sourceDecisionPath?: string;
        }>;
        roles: Readonly<
          Record<"async-esm" | "async-classic", Readonly<{ brotliBytes: number; sha256?: string }>>
        >;
      }>[];
    }>;
    const measured = await Promise.all(
      manifest.assets.map(async (asset) => ({
        brotliBytes: brotliCompressSync(await bytes(asset.file), {
          params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
        }).byteLength,
        compatibleCore: asset.compatible_core,
        file: asset.file,
        role: asset.role,
        sha256: createHash("sha256")
          .update(await bytes(asset.file))
          .digest("hex"),
      })),
    );
    const result = evaluateArtifactBudgets(measured, baseline);
    const esm = measured.find(({ role }) => role === "async-esm")?.brotliBytes;
    const classic = measured.find(({ role }) => role === "async-classic")?.brotliBytes;
    if (esm === undefined || classic === undefined) throw new Error("async_measurement_missing");
    const current = baseline.history[baseline.history.length - 1];
    if (current === undefined) throw new Error("async_baseline_missing");
    const esmBaseline = current.roles["async-esm"].brotliBytes;
    const classicBaseline = current.roles["async-classic"].brotliBytes;
    const esmIncrease = ((esm - esmBaseline) / esmBaseline) * 100;
    const classicIncrease = ((classic - classicBaseline) / classicBaseline) * 100;
    const specification = await readFile(
      new URL(
        "../../docs/specs/suprnova-live/19-developer-tooling-and-testing.md",
        import.meta.url,
      ),
      "utf8",
    );

    expect(result.issues).toEqual([]);
    expect(result.lines).toContain(
      `artifact_budget role=async-esm bytes=${String(esm)} ceiling=none baseline=${String(esmBaseline)} unreviewed_increase=${esmIncrease.toFixed(2)}% threshold=15%`,
    );
    expect(result.lines).toContain(
      `artifact_budget role=async-classic bytes=${String(classic)} ceiling=none baseline=${String(classicBaseline)} unreviewed_increase=${classicIncrease.toFixed(2)}% threshold=15%`,
    );
    expect(current.review).toMatchObject({
      decision: "iteration-004-task-7-membership-budget-policy",
      sourceCommit: "57eb8c260abe44f9aacd8c2cc03b1a54f3ceec61",
      sourceDecision: "iteration-004-task-7-membership-budget-policy",
      sourceDecisionPath: "docs/specs/suprnova-live/19-developer-tooling-and-testing.md",
    });
    expect(
      validateArtifactSizeBaselineProvenance(baseline, new URL("../../", import.meta.url).pathname),
    ).toEqual(baseline);
    const belowThreshold = measured.map((asset) =>
      asset.role === "async-esm"
        ? {
            ...asset,
            brotliBytes: Math.floor(esmBaseline * 1.1),
            sha256: "c".repeat(64),
          }
        : asset,
    );
    expect(evaluateArtifactBudgets(belowThreshold, baseline).issues).toEqual([]);
    const aboveThresholdBytes = Math.floor(esmBaseline * 1.15) + 1;
    const aboveThreshold = belowThreshold.map((asset) =>
      asset.role === "async-esm" ? { ...asset, brotliBytes: aboveThresholdBytes } : asset,
    );
    expect(evaluateArtifactBudgets(aboveThreshold, baseline).issues).toEqual([
      `artifact_budget:async-esm:unreviewed_regression:+${String(aboveThresholdBytes - esmBaseline)}`,
    ]);
    expect(specification).toContain(
      `strictly prior source commit \`57eb8c260abe44f9aacd8c2cc03b1a54f3ceec61\``,
    );
  });

  it("exposes equivalent ESM and non-replaceable classic facades from one singleton core", async () => {
    const esmSource = await bytes("suprnova-live.esm.js");
    const esm = (await import(`data:text/javascript;base64,${esmSource.toString("base64")}`)) as {
      readonly version: string;
      readonly runtimeContractVersion: number;
      readonly supportedProtocolVersions: readonly number[];
      readonly RUNTIME_SYMBOL: symbol;
      readonly boot: unknown;
      readonly lifecycleTestProbe?: unknown;
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
    expect(esm).not.toHaveProperty("lifecycleTestProbe");
    expect(typeof classic.boot).toBe("function");
    expect(descriptor).toMatchObject({ configurable: false, writable: false });
  });

  it("contains no production maps or unsafe/dynamic evaluation forms", async () => {
    for (const name of [
      "suprnova-live.classic.js",
      "suprnova-live.esm.js",
      "suprnova-live.stimulus.classic.js",
      "suprnova-live.stimulus.esm.js",
      "suprnova-live.uploads.classic.js",
      "suprnova-live.uploads.esm.js",
      "suprnova-live.async.classic.js",
      "suprnova-live.async.esm.js",
    ]) {
      const source = (await bytes(name)).toString("utf8");
      if (name === "suprnova-live.classic.js" || name === "suprnova-live.esm.js") {
        expect(source).toContain("Idiomorph 0.7.4 (0BSD)");
      } else {
        expect(source).not.toContain("Idiomorph");
      }
      expect(source).not.toMatch(/from\s*["']idiomorph["']/u);
      expect(source).not.toContain("sourceMappingURL");
      expect(source).not.toMatch(/\beval\s*\(/u);
      expect(source).not.toMatch(/\bnew\s+Function\b/u);
      expect(source).not.toMatch(/\bimport\s*\(/u);
      expect(source).not.toContain("@hotwired/stimulus");
    }
  });

  it("keeps Idiomorph behind the single Live-owned private adapter", async () => {
    const sourceRoot = new URL("../src/", import.meta.url);
    const sourceNames = (await readdir(sourceRoot, { recursive: true })).filter((name) =>
      name.endsWith(".ts"),
    );
    const idiomorphImports: string[] = [];
    let adapterSource = "";
    for (const name of sourceNames) {
      const source = await readFile(new URL(name, sourceRoot), "utf8");
      if (/from\s+["']idiomorph["']/u.test(source)) idiomorphImports.push(name);
      if (name === "morph/idiomorph.ts") adapterSource = source;
    }

    expect(idiomorphImports).toEqual(["morph/idiomorph.ts"]);
    expect(adapterSource).toContain('morphStyle: "outerHTML"');
  });

  it("rebuilds every production byte identically", async () => {
    const second = join(temporaryRoot, "second");
    runBuild(second);
    const names = (await readdir(outputDirectory)).sort();
    expect((await readdir(second)).sort()).toEqual(names);
    for (const name of names) {
      expect(await readFile(join(second, name))).toEqual(await bytes(name));
    }
  }, 15_000);
});
