import { execFile } from "node:child_process";
import { link, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

import { describe, expect, it } from "vitest";

import {
  assertUploadArtifactNamespace,
  assertUploadBenchmarkBundleInputs,
  estimateUploadManagerOwnedBytes,
  UploadTransferChunkObserver,
} from "../benchmarks/upload-accounting.js";
import {
  ControlledImmediateReceiptWaves,
  ImmediateUploadTransport,
} from "../benchmarks/upload-workloads.js";
import { argumentsFrom, atomicWriteEvidence } from "../scripts/run-upload-budget.mjs";

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
    const slots = Array.from({ length: 4 }, (_, index) => index);
    const observer = new UploadTransferChunkObserver();
    observer.observe(
      slots.map((slot) => ({ buffers: 1, bytes: 256 * 1024, slot })),
      [
        { buffers: 2, bytes: 2 * 256 * 1024, slot: slots[0] ?? -1 },
        { buffers: 0, bytes: 0, slot: slots[1] ?? -1 },
        { buffers: 1, bytes: 256 * 1024, slot: slots[2] ?? -1 },
        { buffers: 1, bytes: 256 * 1024, slot: slots[3] ?? -1 },
      ],
    );
    const snapshot = observer.snapshot();
    expect(snapshot.liveChunkBuffers).toBe(8);
    expect(snapshot.liveChunkBuffers / slots.length).toBe(2);
    expect(snapshot.maxChunksPerTransfer).toBe(3);
    expect(snapshot.chunkBuffersByTransfer[0]?.totalHighWater).toBe(3);
  });

  it("tracks every same-handle transport operation by token through out-of-order completion", async () => {
    const releases = new Map<string, () => void>();
    const transport = new ImmediateUploadTransport(
      (_request, token) =>
        new Promise<void>((resolve) => {
          releases.set(token, resolve);
        }),
    );
    const created = await transport.send({ operation: "create" });
    const handle = created.handle;
    if (handle === undefined) throw new Error("fixture_handle_missing");
    const pending = Array.from({ length: 3 }, () =>
      transport.send({
        bytes: new ArrayBuffer(256 * 1024),
        expectedRevision: "1",
        handle,
        operation: "put_chunk",
      }),
    );
    await Promise.resolve();

    expect(releases.size).toBe(3);
    expect(transport.activeChunksByTransfer()).toEqual([
      { buffers: 3, bytes: 3 * 256 * 1024, slot: 0 },
    ]);
    expect(transport.maximumConcurrentOperations()).toBe(3);
    expect(transport.maximumConcurrentTransfers()).toBe(1);

    const tokens = [...releases.keys()];
    releases.get(tokens[1] ?? "")?.();
    await pending[1];
    expect(transport.activeChunksByTransfer()).toEqual([
      { buffers: 2, bytes: 2 * 256 * 1024, slot: 0 },
    ]);
    releases.get(tokens[0] ?? "")?.();
    releases.get(tokens[2] ?? "")?.();
    await Promise.all(pending);
    expect(transport.activeChunksByTransfer()).toEqual([]);
    expect(transport.activeOperationCount()).toBe(0);
  });

  it("holds controlled immediate receipts until four distinct transfers are in flight", async () => {
    const receipts = new ControlledImmediateReceiptWaves(4);
    const transport = new ImmediateUploadTransport((request, token) =>
      receipts.wait(request, token),
    );
    const creations = await Promise.all(
      Array.from({ length: 4 }, () => transport.send({ operation: "create" })),
    );
    const pending = creations.map(({ handle }) => {
      if (handle === undefined) throw new Error("fixture_handle_missing");
      return transport.send({
        bytes: new ArrayBuffer(256 * 1024),
        expectedRevision: "1",
        handle,
        operation: "put_chunk",
      });
    });
    await Promise.resolve();

    expect(transport.maximumConcurrentOperations()).toBe(4);
    expect(transport.maximumConcurrentTransfers()).toBe(4);
    await Promise.all(pending);
    expect(transport.activeOperationCount()).toBe(0);
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

  it("bounds a never-completing artifact workload and closes its browser context", async () => {
    const output = await nativeRunner(`
      import { chromium } from "@playwright/test";
      import { bundledModule, measureRun } from ${JSON.stringify(runnerUrl)};
      const workload = await bundledModule(
        "benchmarks/upload-workloads.ts",
        "browser",
        "iife",
        "SuprnovaUploadBudget",
      );
      const neverCompletes = \`
        export function configureUploads() {}
        export const uploadsFeature = Object.freeze([
          Symbol.for("suprnova.live.feature.v1"),
          0,
          1,
          1099511758848,
          Object.freeze({}),
          () => true,
        ]);
      \`;
      const browser = await chromium.launch({ headless: true });
      try {
        try {
          await measureRun(browser, neverCompletes, workload.source, { watchdogMilliseconds: 25 });
          throw new Error("never_completing_workload_succeeded");
        } catch (error) {
          if (!(error instanceof Error) || !error.message.includes("upload_budget_browser_watchdog")) {
            throw error;
          }
        }
        if (browser.contexts().length !== 0) throw new Error("browser_context_leaked");
        process.stdout.write("watchdog-clean");
      } finally {
        await browser.close();
      }
    `);
    expect(output).toBe("watchdog-clean");
  });

  it("writes evidence atomically and rejects canonical baseline aliases", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-upload-budget-"));
    try {
      const baselineDirectory = join(root, "checked");
      const baseline = join(baselineDirectory, "baseline.json");
      const output = join(root, "candidate.json");
      await writeFile(baseline, "baseline\n", { encoding: "utf8", flag: "wx" }).catch(
        async (error: unknown) => {
          if (!(error instanceof Error) || !("code" in error) || error.code !== "ENOENT")
            throw error;
          const { mkdir } = await import("node:fs/promises");
          await mkdir(baselineDirectory, { recursive: true });
          await writeFile(baseline, "baseline\n", { encoding: "utf8", flag: "wx" });
        },
      );
      await writeFile(output, "old\n", "utf8");

      await expect(
        atomicWriteEvidence(output, "new\n", baseline, { failStage: "after_partial_write" }),
      ).rejects.toThrow("evidence_write_failed");
      expect(await readFile(output, "utf8")).toBe("old\n");
      expect(await readFile(baseline, "utf8")).toBe("baseline\n");

      await expect(
        atomicWriteEvidence(output, "new\n", baseline, { failStage: "before_rename" }),
      ).rejects.toThrow("evidence_rename_failed");
      expect(await readFile(output, "utf8")).toBe("old\n");

      const hardlink = join(root, "hardlink.json");
      await link(baseline, hardlink);
      await expect(atomicWriteEvidence(hardlink, "new\n", baseline)).rejects.toThrow(
        "baseline_overwrite_forbidden",
      );

      const aliasParent = join(root, "alias");
      await symlink(baselineDirectory, aliasParent, "dir");
      await expect(
        atomicWriteEvidence(join(aliasParent, "baseline.json"), "new\n", baseline),
      ).rejects.toThrow("baseline_overwrite_forbidden");

      const symlinkOutput = join(root, "symlink.json");
      await symlink(baseline, symlinkOutput);
      await expect(atomicWriteEvidence(symlinkOutput, "new\n", baseline)).rejects.toThrow(
        "baseline_overwrite_forbidden",
      );

      await atomicWriteEvidence(output, "new\n", baseline);
      expect(await readFile(output, "utf8")).toBe("new\n");
      expect(await readFile(baseline, "utf8")).toBe("baseline\n");
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });
});
