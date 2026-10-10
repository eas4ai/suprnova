import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";

import { build } from "esbuild";
import { describe, expect, it } from "vitest";

import { ASYNC_BUDGET_DRIVER_MARKER } from "../benchmarks/async-budget-workloads.js";
import {
  AsyncBudgetRunnerError,
  argumentsFrom,
  childExecutionFailure,
  exactServerEvidence,
  verifyArtifactBinding,
  withAsyncBudgetBrowserResources,
  withAsyncBudgetPageResources,
} from "../scripts/run-async-budget.mjs";

const SHA256 = "a".repeat(64);

describe("E100/1K and R100 benchmark integrity", () => {
  it("permits exploratory recording only for the reduced profile", () => {
    expect(argumentsFrom(["--record-exploratory"]).recordExploratory).toBe(true);
    expect(argumentsFrom(["--verify-retention-mutations"]).verifyRetentionMutations).toBe(true);
    expect(argumentsFrom(["--retention-mutation", "stale_queued_payload"]).retentionMutation).toBe(
      "stale_queued_payload",
    );
    expect(() => argumentsFrom(["--retention-mutation", "guessed_bytes"])).toThrow(
      "retention_mutation_invalid",
    );
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

  it("closes the artifact server when browser launch rejects", async () => {
    const events: string[] = [];
    const primary = new AsyncBudgetRunnerError("launch_rejected");
    await expect(
      withAsyncBudgetBrowserResources(
        {
          closeBrowser: () => {
            events.push("browser:close");
            return Promise.resolve();
          },
          closeContext: () => {
            events.push("context:close");
            return Promise.resolve();
          },
          closeServer: () => {
            events.push("server:close");
            return Promise.resolve();
          },
          createServer: () => {
            events.push("server:create");
            return {};
          },
          launch: () => {
            events.push("browser:launch");
            return Promise.reject(primary);
          },
          listen: () => {
            events.push("server:listen");
            return Promise.resolve("http://127.0.0.1:1");
          },
          newContext: () => {
            events.push("context:create");
            return Promise.resolve({});
          },
        },
        () => Promise.resolve(undefined),
      ),
    ).rejects.toMatchObject({ code: "launch_rejected" });
    expect(events).toEqual(["server:create", "server:listen", "browser:launch", "server:close"]);
  });

  it("releases the real artifact-server port when launch rejects", async () => {
    let port = 0;
    await expect(
      withAsyncBudgetBrowserResources(
        {
          closeBrowser: () => Promise.resolve(),
          closeContext: () => Promise.resolve(),
          closeServer: (server) =>
            new Promise<void>((resolvePromise, reject) => {
              server.close((error) => {
                if (error === undefined) resolvePromise();
                else reject(error);
              });
            }),
          createServer: () => createServer(),
          launch: () => Promise.reject(new AsyncBudgetRunnerError("launch_rejected")),
          listen: (server) =>
            new Promise<string>((resolvePromise, reject) => {
              server.once("error", reject);
              server.listen(0, "127.0.0.1", () => {
                const address = server.address();
                if (typeof address !== "object" || address === null) {
                  reject(new Error());
                  return;
                }
                port = address.port;
                resolvePromise(`http://127.0.0.1:${String(port)}`);
              });
            }),
          newContext: () => Promise.resolve({}),
        },
        () => Promise.resolve(undefined),
      ),
    ).rejects.toMatchObject({ code: "launch_rejected" });

    const probe = createServer();
    await new Promise<void>((resolvePromise, reject) => {
      probe.once("error", reject);
      probe.listen(port, "127.0.0.1", resolvePromise);
    });
    await new Promise<void>((resolvePromise, reject) => {
      probe.close((error) => {
        if (error === undefined) resolvePromise();
        else reject(error);
      });
    });
  });

  it("attempts every outer close when browser close rejects", async () => {
    const events: string[] = [];
    await expect(
      withAsyncBudgetBrowserResources(
        {
          closeBrowser: () => {
            events.push("browser:close");
            return Promise.reject(new Error("browser close rejected"));
          },
          closeContext: () => {
            events.push("context:close");
            return Promise.resolve();
          },
          closeServer: () => {
            events.push("server:close");
            return Promise.resolve();
          },
          createServer: () => ({}),
          launch: () => Promise.resolve({}),
          listen: () => Promise.resolve("http://127.0.0.1:1"),
          newContext: () => Promise.resolve({}),
        },
        () => Promise.resolve("measured"),
      ),
    ).rejects.toMatchObject({ code: "async_budget_cleanup_failed" });
    expect(events).toEqual(["context:close", "browser:close", "server:close"]);
  });

  it("closes a created page when partial CDP setup rejects", async () => {
    const events: string[] = [];
    const primary = new AsyncBudgetRunnerError("cdp_setup_rejected");
    await expect(
      withAsyncBudgetPageResources(
        {},
        {
          closePage: () => {
            events.push("page:close");
            return Promise.resolve();
          },
          detachSession: () => {
            events.push("session:detach");
            return Promise.resolve();
          },
          newPage: () => {
            events.push("page:create");
            return Promise.resolve({});
          },
          newSession: () => {
            events.push("session:create");
            return Promise.reject(primary);
          },
        },
        () => Promise.resolve(undefined),
      ),
    ).rejects.toMatchObject({ code: "cdp_setup_rejected" });
    expect(events).toEqual(["page:create", "session:create", "page:close"]);
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
    expect(source).toContain("closeBrowser: (browser) => browser.close()");
    expect(source).toContain("closeServer,");
    expect(source).not.toMatch(/setTimeout\([^)]*measureAsyncWorkloads/u);
  });

  it("measures retained runtime heap through forced-GC Chromium CDP without guessed bytes", async () => {
    const [runner, helper, driver] = await Promise.all([
      readFile(new URL("../scripts/run-async-budget.mjs", import.meta.url), "utf8"),
      readFile(new URL("../benchmarks/async-budget-workloads.ts", import.meta.url), "utf8"),
      readFile(new URL("../benchmarks/async-workloads.ts", import.meta.url), "utf8"),
    ]);
    expect(runner).toContain('session.send("HeapProfiler.collectGarbage")');
    expect(runner).toContain('session.send("Runtime.getHeapUsage")');
    expect(runner).toContain('session.send("Browser.getVersion")');
    expect(runner).toContain("predecessorTransportOwners");
    expect(runner).toContain("predecessorContinuityOwners");
    expect(runner).toContain("postWorkload");
    expect(runner).toContain("derivePostWorkloadRetention");
    expect(runner).toContain("large_island_buffer");
    expect(runner).toContain("stale_current_payload");
    expect(runner).toContain("stale_queued_payload");
    expect(runner).not.toContain("SUPRNOVA_LIVE_ASYNC_DEBUG_EVIDENCE");
    expect(runner).not.toContain(".disconnect(subscriptionIndex)");
    expect(runner).not.toContain(".connect(subscriptionIndex)");
    expect(helper).not.toContain("__suprnovaBenchmarkRetainedMutation");
    expect(helper).not.toContain("currentPayloadMutationOwners");
    expect(helper).not.toContain("queuedPayloadMutationOwners");
    expect(driver).toContain("currentInFlightRefreshes");
    expect(driver).toContain("queuedPayloadBytes");
    expect(driver).not.toContain("currentInFlightRefreshes[index] = 0");
    expect(driver).not.toContain("queuedPayloadBytes[index] = 0");
    expect(helper).not.toMatch(/pendingEvents\s*\*|pollTimers\s*\*|runtimeRecords\s*\*/u);
    expect(helper).not.toContain("estimateAsyncRetainedBytes");
  });
});
