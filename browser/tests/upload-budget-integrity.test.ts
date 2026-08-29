import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { promisify } from "node:util";

import { describe, expect, it } from "vitest";

import {
  assertUploadArtifactNamespace,
  assertUploadBenchmarkBundleInputs,
  estimateUploadManagerOwnedBytes,
  UploadTransferChunkObserver,
} from "../benchmarks/upload-accounting.js";
import { argumentsFrom } from "../scripts/run-upload-budget.mjs";

const execFileAsync = promisify(execFile);
const runnerUrl = new URL("../scripts/run-upload-budget.mjs", import.meta.url).href;

async function nativeRunner(source: string): Promise<string> {
  const result = await execFileAsync(process.execPath, ["--input-type=module", "--eval", source], {
    cwd: new URL("..", import.meta.url),
  });
  return result.stdout;
}

describe("U4/16 benchmark integrity", () => {
  it("permits exploratory recording only for the unqualified profile", () => {
    expect(argumentsFrom(["--record-exploratory"]).recordExploratory).toBe(true);
    expect(() => argumentsFrom(["--profile", "qualified", "--record-exploratory"])).toThrow(
      "exploratory_record_requires_reduced_profile",
    );
  });

  it("rejects a benchmark bundle containing a second upload implementation", () => {
    expect(() => {
      assertUploadBenchmarkBundleInputs([
        "benchmarks/upload-workloads.ts",
        "src/uploads/manager.ts",
      ]);
    }).toThrow("upload_budget_bundle_contains_production_implementation");
    expect(() => {
      assertUploadBenchmarkBundleInputs([
        "benchmarks/upload-workloads.ts",
        "src/uploads/transfer.ts",
      ]);
    }).toThrow("upload_budget_bundle_contains_production_implementation");
    expect(() => {
      assertUploadBenchmarkBundleInputs([
        "benchmarks/upload-workloads.ts",
        "src/uploads/progress.ts",
      ]);
    }).toThrow("upload_budget_bundle_contains_production_implementation");
    expect(() => {
      assertUploadBenchmarkBundleInputs([
        "benchmarks/upload-workloads.ts",
        "benchmarks/upload-accounting.ts",
        "benchmarks/upload-schema.ts",
      ]);
    }).not.toThrow();
  });

  it("fails before measuring when the production artifact surface is corrupt", () => {
    expect(() => {
      assertUploadArtifactNamespace(Object.freeze({}));
    }).toThrow("upload_budget_artifact_surface_invalid");
    expect(() => {
      assertUploadArtifactNamespace(
        Object.freeze({
          configureUploads() {
            return undefined;
          },
          uploadsFeature: Object.freeze([Symbol(), 0, 1, 1, Object.freeze({}), () => false]),
        }),
      );
    }).toThrow("upload_budget_artifact_surface_invalid");
  });

  it("derives manager bytes from observed owned categories", () => {
    const base = {
      activeLeases: 4,
      bindings: 1,
      cleanupObligations: 0,
      entries: 4,
      generationFields: 1,
      observers: 1,
      ownedResources: 4,
      pendingChunkBytes: 0,
      pendingChunkBuffers: 0,
      queuedBytes: 0,
      queuedItems: 0,
      retainedStringCodeUnits: 512,
      waitingPermits: 0,
    } as const;
    const observed = estimateUploadManagerOwnedBytes(base);
    expect(estimateUploadManagerOwnedBytes({ ...base, queuedItems: 4 })).toBeGreaterThan(observed);
    expect(
      estimateUploadManagerOwnedBytes({ ...base, retainedStringCodeUnits: 200_000 }),
    ).toBeGreaterThan(256 * 1024);
  });

  it("detects three buffers held by one transfer while the document average remains two", () => {
    const handles = Array.from(
      { length: 4 },
      (_, index) => `018f47c1-2af0-7cc4-a001-${String(index + 1).padStart(12, "0")}`,
    );
    const observer = new UploadTransferChunkObserver();
    observer.observe(
      handles.map((handle) => ({ buffers: 1, bytes: 256 * 1024, handle })),
      [
        { buffers: 2, bytes: 2 * 256 * 1024, handle: handles[0] ?? "" },
        { buffers: 0, bytes: 0, handle: handles[1] ?? "" },
        { buffers: 1, bytes: 256 * 1024, handle: handles[2] ?? "" },
        { buffers: 1, bytes: 256 * 1024, handle: handles[3] ?? "" },
      ],
    );
    const snapshot = observer.snapshot();
    expect(snapshot.liveChunkBuffers).toBe(8);
    expect(snapshot.liveChunkBuffers / handles.length).toBe(2);
    expect(snapshot.maxChunksPerTransfer).toBe(3);
    expect(snapshot.chunkBuffersByTransfer[0]?.totalHighWater).toBe(3);
  });

  it("loads the artifact namespace and passes it into the workload", async () => {
    const runner = await readFile(
      new URL("../scripts/run-upload-budget.mjs", import.meta.url),
      "utf8",
    );
    const workload = await readFile(
      new URL("../benchmarks/upload-workloads.ts", import.meta.url),
      "utf8",
    );
    expect(runner).toContain("artifactNamespace");
    expect(runner).toContain("measureU4_16(artifactNamespace)");
    expect(workload).not.toMatch(/\.\.\/src\/uploads\/(?:manager|transfer|progress)\.js/u);
  });

  it("bundles no duplicate production implementation into the benchmark IIFE", async () => {
    const inputs = JSON.parse(
      await nativeRunner(`
        import { bundledModule } from ${JSON.stringify(runnerUrl)};
        const bundle = await bundledModule(
          "benchmarks/upload-workloads.ts",
          "browser",
          "iife",
          "SuprnovaUploadBudget",
        );
        process.stdout.write(JSON.stringify(bundle.inputs));
      `),
    ) as string[];
    expect(() => {
      assertUploadBenchmarkBundleInputs(inputs);
    }).not.toThrow();
    expect(inputs).not.toEqual(
      expect.arrayContaining([
        expect.stringMatching(/src\/uploads\/(?:manager|progress|transfer)\.ts$/u),
      ]),
    );
  });

  it("fails when the measured artifact keeps the ABI but corrupts production drive behavior", async () => {
    const output = await nativeRunner(`
      import { chromium } from "@playwright/test";
      import { bundledModule, measureRun } from ${JSON.stringify(runnerUrl)};
      const workload = await bundledModule(
        "benchmarks/upload-workloads.ts",
        "browser",
        "iife",
        "SuprnovaUploadBudget",
      );
      const corruptArtifact = \`
        export function configureUploads() {}
        export const uploadsFeature = Object.freeze([
          Symbol.for("suprnova.live.feature.v1"),
          0,
          1,
          1099511758848,
          Object.freeze({}),
          () => false,
        ]);
      \`;
      const browser = await chromium.launch({ headless: true });
      try {
        await measureRun(browser, corruptArtifact, workload.source);
        throw new Error("corrupt_artifact_was_measured_successfully");
      } catch (error) {
        if (!(error instanceof Error) || !error.message.includes("upload_budget_artifact_drive_failed")) {
          throw error;
        }
        process.stdout.write("corruption-rejected");
      } finally {
        await browser.close();
      }
    `);
    expect(output).toBe("corruption-rejected");
  });
});
