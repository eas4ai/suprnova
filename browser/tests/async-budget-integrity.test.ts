import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

import { build } from "esbuild";
import { describe, expect, it } from "vitest";

import { ASYNC_BUDGET_DRIVER_MARKER } from "../benchmarks/async-budget-workloads.js";
import {
  argumentsFrom,
  childExecutionFailure,
  exactServerEvidence,
  verifyArtifactBinding,
} from "../scripts/run-async-budget.mjs";

const SHA256 = "a".repeat(64);

describe("E100/1K and R100 benchmark integrity", () => {
  it("permits exploratory recording only for the reduced profile", () => {
    expect(argumentsFrom(["--record-exploratory"]).recordExploratory).toBe(true);
    expect(() => argumentsFrom(["--profile", "qualified", "--record-exploratory"])).toThrow(
      "exploratory_record_requires_reduced_profile",
    );
    expect(() => argumentsFrom(["--output", "same", "--baseline", "same"])).toThrow(
      "baseline_overwrite_forbidden",
    );
  });

  it("binds the exact production artifact and rejects byte corruption", () => {
    const artifact = Buffer.from("production-async-artifact", "utf8");
    const manifest = Buffer.from(
      JSON.stringify({
        assets: [
          {
            file: "suprnova-live.async.esm.js",
            role: "async-esm",
            sha256: createHash("sha256").update(artifact).digest("hex"),
          },
        ],
      }),
      "utf8",
    );
    expect(verifyArtifactBinding(artifact, manifest).sha256).toHaveLength(64);
    expect(() => verifyArtifactBinding(Buffer.from(artifact).fill(0, 0, 1), manifest)).toThrow(
      "artifact_manifest_mismatch",
    );
  });

  it("keeps benchmark observers outside the production async metafile", async () => {
    const production = await build({
      absWorkingDir: new URL("..", import.meta.url).pathname,
      bundle: true,
      entryPoints: ["src/entry-async-esm.ts"],
      format: "esm",
      legalComments: "none",
      metafile: true,
      minify: true,
      platform: "browser",
      target: ["chrome111"],
      write: false,
    });
    const driver = await build({
      absWorkingDir: new URL("..", import.meta.url).pathname,
      bundle: true,
      entryPoints: ["benchmarks/async-budget-driver.ts"],
      format: "iife",
      globalName: "SuprnovaAsyncBudgetDriver",
      legalComments: "none",
      metafile: true,
      minify: true,
      platform: "browser",
      target: ["chrome111"],
      write: false,
    });
    const productionSource = production.outputFiles[0]?.text ?? "";
    const driverSource = driver.outputFiles[0]?.text ?? "";
    expect(productionSource).not.toContain(ASYNC_BUDGET_DRIVER_MARKER);
    expect(driverSource).toContain(ASYNC_BUDGET_DRIVER_MARKER);
    expect(Object.keys(production.metafile.inputs)).not.toEqual(
      expect.arrayContaining([expect.stringMatching(/benchmarks\/async-budget/u)]),
    );
    expect(Object.keys(driver.metafile.inputs)).not.toEqual(
      expect.arrayContaining([expect.stringMatching(/src\/async-updates/u)]),
    );
  });

  it("rejects corrupt or artifact-detached Rust owner evidence", () => {
    const valid = {
      artifactSha256: SHA256,
      evidence: {
        dispatches: 1_100,
        fairnessMaximumLead: 1,
        finalCurrentSubscriptions: 100,
        logicalMemberships: 100,
        maxQueuedBytes: 32_768,
        maxQueuedEvents: 32,
        physicalDocumentTransports: 1,
        providerPath: "BoundedDocumentTransportSession",
        sequenceMismatches: 0,
      },
      processId: 42,
      schemaVersion: 1,
      suite: "E100/1K",
    };
    expect(exactServerEvidence(valid, SHA256)).toEqual(valid.evidence);
    expect(() => exactServerEvidence({ ...valid, artifactSha256: "b".repeat(64) }, SHA256)).toThrow(
      "async_server_evidence_invalid",
    );
    expect(() => exactServerEvidence({ ...valid, evidence: null }, SHA256)).toThrow(
      "async_server_evidence_invalid",
    );
  });

  it("classifies child-process timeouts as watchdog failures", () => {
    expect(
      childExecutionFailure(
        { error: { code: "ETIMEDOUT" }, status: null },
        "watchdog",
        "child_failed",
      ),
    ).toBe("watchdog");
    expect(childExecutionFailure({ status: 1 }, "watchdog", "child_failed")).toBe("child_failed");
    expect(childExecutionFailure({ status: 0 }, "watchdog", "child_failed")).toBeNull();
  });

  it("uses the shared atomic writer and keeps watchdogs outside measured samples", async () => {
    const source = await readFile(
      new URL("../scripts/run-async-budget.mjs", import.meta.url),
      "utf8",
    );
    expect(source).toContain("atomicWriteEvidence");
    expect(source).toContain("timeout: CHILD_TIMEOUT_MILLISECONDS");
    expect(source).toContain("async_budget_watchdog");
    expect(source).toContain("async_server_proof_watchdog");
    expect(source).toContain("await browser.close()");
    expect(source).toContain("await closeServer(server)");
    expect(source).not.toMatch(/setTimeout\([^)]*measureAsyncWorkloads/u);
  });
});
