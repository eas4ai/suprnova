import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { collectUploadServerRuns } from "../scripts/run-upload-server-processes.mjs";

function rawRun(runIndex: number, processId: number): Record<string, unknown> {
  return { processId, runIndex };
}

describe("upload server process runner", () => {
  it("captures exactly three distinct Rust process results for a qualified run", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-upload-server-runs-"));
    const destination = join(root, "server-runs.json");
    const invocations: { cpuSet: string; profile: string; resultPath: string; runIndex: number }[] =
      [];
    try {
      await collectUploadServerRuns({
        destination,
        profile: "qualified",
        runProcess: async ({ cpuSet, profile, resultPath, runIndex }) => {
          invocations.push({ cpuSet, profile, resultPath, runIndex });
          await writeFile(resultPath, `${JSON.stringify(rawRun(runIndex, 10_000 + runIndex))}\n`);
        },
      });

      expect(invocations.map(({ runIndex }) => runIndex)).toEqual([1, 2, 3]);
      expect(invocations.every(({ cpuSet }) => cpuSet === "0-7")).toBe(true);
      expect(invocations.every(({ profile }) => profile === "qualified")).toBe(true);
      expect(new Set(invocations.map(({ resultPath }) => resultPath)).size).toBe(3);
      const envelope = JSON.parse(await readFile(destination, "utf8")) as {
        processRuns: { processId: number; runIndex: number }[];
        schemaVersion: number;
      };
      expect(envelope.schemaVersion).toBe(1);
      expect(envelope.processRuns.map(({ runIndex }) => runIndex)).toEqual([1, 2, 3]);
      expect(new Set(envelope.processRuns.map(({ processId }) => processId)).size).toBe(3);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  it("keeps existing evidence byte-identical and cleans temporary runs when run two fails", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-upload-server-failure-"));
    const destination = join(root, "server-runs.json");
    const original = "reviewed evidence\n";
    await writeFile(destination, original);
    try {
      await expect(
        collectUploadServerRuns({
          destination,
          profile: "qualified",
          runProcess: async ({ resultPath, runIndex }) => {
            if (runIndex === 2) throw new Error("injected_run_two_failure");
            await writeFile(resultPath, `${JSON.stringify(rawRun(runIndex, 20_000 + runIndex))}\n`);
          },
        }),
      ).rejects.toThrow("injected_run_two_failure");

      expect(await readFile(destination, "utf8")).toBe(original);
      expect((await readdir(root)).sort()).toEqual(["server-runs.json"]);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  it("keeps exploratory mode to one explicitly unqualified process run", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-upload-server-exploratory-"));
    const destination = join(root, "server-runs.json");
    const invocations: number[] = [];
    try {
      await collectUploadServerRuns({
        destination,
        profile: "reduced",
        runProcess: async ({ resultPath, runIndex }) => {
          invocations.push(runIndex);
          await writeFile(resultPath, `${JSON.stringify(rawRun(runIndex, 30_000 + runIndex))}\n`);
        },
      });
      expect(invocations).toEqual([1]);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });

  it("refuses to use the checked baseline as its intermediate process envelope", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-upload-server-baseline-"));
    const baseline = join(root, "upload-budget-v1.json");
    const original = "reviewed baseline\n";
    await writeFile(baseline, original);
    try {
      await expect(
        collectUploadServerRuns({
          baseline,
          destination: baseline,
          profile: "reduced",
          runProcess: async ({ resultPath, runIndex }) => {
            await writeFile(resultPath, `${JSON.stringify(rawRun(runIndex, 40_001))}\n`);
          },
        }),
      ).rejects.toThrow("baseline_overwrite_forbidden");
      expect(await readFile(baseline, "utf8")).toBe(original);
      expect((await readdir(root)).sort()).toEqual(["upload-budget-v1.json"]);
    } finally {
      await rm(root, { force: true, recursive: true });
    }
  });
});
