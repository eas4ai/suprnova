import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { mkdir, mkdtemp, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterAll, describe, expect, it } from "vitest";

interface MatrixTarget {
  readonly id: string;
  readonly browserProduct: "chrome" | "edge" | "firefox" | "safari";
  readonly channel: "minimum" | "stable";
  readonly version: string;
}

interface CompatibilityMatrix {
  readonly schemaVersion: 1;
  readonly evidenceSchema: string;
  readonly runtimeArtifact: string;
  readonly fixtureManifest: string;
  readonly maxEvidenceAgeDays: number;
  readonly requiredCases: readonly string[];
  readonly targets: readonly MatrixTarget[];
}

interface CheckResult {
  readonly status: "qualified" | "failed" | "unqualified";
  readonly qualified: number;
  readonly required: number;
  readonly details: readonly { readonly target: string; readonly code: string }[];
}

interface EvidenceFixture {
  readonly root: string;
  readonly matrixPath: string;
  readonly resultsPath: string;
  readonly runtimeSha256: string;
  readonly fixtureManifestSha256: string;
}

const EXPECTED_TARGETS = Object.freeze([
  { id: "chrome-minimum-111", browserProduct: "chrome", channel: "minimum", version: "111" },
  { id: "chrome-current-stable", browserProduct: "chrome", channel: "stable", version: "current" },
  { id: "edge-minimum-111", browserProduct: "edge", channel: "minimum", version: "111" },
  { id: "edge-current-stable", browserProduct: "edge", channel: "stable", version: "current" },
  { id: "firefox-minimum-128", browserProduct: "firefox", channel: "minimum", version: "128" },
  {
    id: "firefox-current-stable",
    browserProduct: "firefox",
    channel: "stable",
    version: "current",
  },
  { id: "safari-minimum-16-4", browserProduct: "safari", channel: "minimum", version: "16.4" },
  { id: "safari-current-stable", browserProduct: "safari", channel: "stable", version: "current" },
] satisfies readonly MatrixTarget[]);

const CHECKER = fileURLToPath(new URL("../scripts/check-compatibility.mjs", import.meta.url));
const RUNNER = fileURLToPath(new URL("../scripts/run-compatibility.mjs", import.meta.url));
const NOW = "2026-08-22T18:00:00.000Z";
const temporaryDirectories: string[] = [];

async function matrix(): Promise<CompatibilityMatrix> {
  const source = await readFile(new URL("../compatibility/matrix.json", import.meta.url), "utf8");
  return JSON.parse(source) as CompatibilityMatrix;
}

function requiredTarget(id: string): MatrixTarget {
  const target = EXPECTED_TARGETS.find((candidate) => candidate.id === id);
  if (target === undefined) {
    throw new Error(`Missing required compatibility target: ${id}`);
  }
  return target;
}

async function evidenceFixture(): Promise<EvidenceFixture> {
  const root = await mkdtemp(join(tmpdir(), "suprnova-live-compatibility-"));
  temporaryDirectories.push(root);
  const directory = join(root, "compatibility");
  const resultsPath = join(directory, "results");
  await mkdir(resultsPath, { recursive: true });
  const sourceMatrix = await matrix();
  const runtime = Buffer.from("current production runtime", "utf8");
  const fixtureManifestSha256 = "b".repeat(64);
  await writeFile(join(root, "runtime.js"), runtime);
  await writeFile(join(root, "manifest.sha256"), `${fixtureManifestSha256}\n`, "utf8");
  await writeFile(
    join(directory, "schema.json"),
    await readFile(new URL("../compatibility/schema.json", import.meta.url), "utf8"),
    "utf8",
  );
  await writeFile(
    join(directory, "matrix.json"),
    `${JSON.stringify(
      {
        ...sourceMatrix,
        evidenceSchema: "./schema.json",
        runtimeArtifact: "../runtime.js",
        fixtureManifest: "../manifest.sha256",
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  return {
    root,
    matrixPath: join(directory, "matrix.json"),
    resultsPath,
    runtimeSha256: createHash("sha256").update(runtime).digest("hex"),
    fixtureManifestSha256,
  };
}

function evidenceFor(target: MatrixTarget, fixture: EvidenceFixture, result = "pass") {
  const stableVersions = { chrome: "140.0.0", edge: "140.0.0", firefox: "142.0", safari: "18.6" };
  return {
    schemaVersion: 1,
    browserProduct: target.browserProduct,
    browserVersion:
      target.channel === "minimum" ? target.version : stableVersions[target.browserProduct],
    operatingSystem: target.browserProduct === "safari" ? "macOS 15.6" : "Provider OS 2026.08",
    provider: "Actual Browser Qualification Lab",
    runtimeSha256: fixture.runtimeSha256,
    fixtureManifestSha256: fixture.fixtureManifestSha256,
    executedAt: NOW,
    result,
    attestation: `urn:suprnova-live:actual-browser:${target.id}:attested`,
  };
}

async function writeCompleteEvidence(fixture: EvidenceFixture): Promise<void> {
  for (const target of EXPECTED_TARGETS) {
    await writeFile(
      join(fixture.resultsPath, `${target.id}.json`),
      `${JSON.stringify(evidenceFor(target, fixture), null, 2)}\n`,
      "utf8",
    );
  }
}

function check(fixture: EvidenceFixture) {
  const execution = spawnSync(
    process.execPath,
    [
      CHECKER,
      "--matrix",
      fixture.matrixPath,
      "--results",
      fixture.resultsPath,
      "--now",
      NOW,
      "--json",
    ],
    { encoding: "utf8" },
  );
  const result = JSON.parse(execution.stdout) as CheckResult;
  return { process: execution, result };
}

afterAll(async () => {
  await Promise.all(
    temporaryDirectories.map(async (directory) => rm(directory, { recursive: true })),
  );
});

describe("actual-browser compatibility evidence", () => {
  it("requires every minimum floor and current stable channel by actual product name", async () => {
    const value = await matrix();
    expect(value.schemaVersion).toBe(1);
    expect(value.targets).toEqual(EXPECTED_TARGETS);
    expect(value.requiredCases.length).toBeGreaterThan(20);
    expect(new Set(value.requiredCases).size).toBe(value.requiredCases.length);
  });

  it("ships a closed schema that rejects product aliases and incomplete evidence", async () => {
    const source = await readFile(new URL("../compatibility/schema.json", import.meta.url), "utf8");
    const schema = JSON.parse(source) as {
      readonly additionalProperties: boolean;
      readonly required: readonly string[];
      readonly properties: {
        readonly browserProduct: { readonly enum: readonly string[] };
      };
    };
    expect(schema.additionalProperties).toBe(false);
    expect(schema.required).toHaveLength(10);
    expect(schema.properties.browserProduct.enum).toEqual(["chrome", "edge", "firefox", "safari"]);
    expect(schema.properties.browserProduct.enum).not.toContain("chromium");
    expect(schema.properties.browserProduct.enum).not.toContain("webkit");
  });

  it("distinguishes qualified, failed, and unqualified without accepting stale identities", async () => {
    const fixture = await evidenceFixture();
    await writeCompleteEvidence(fixture);

    const qualified = check(fixture);
    expect(qualified.process.status).toBe(0);
    expect(qualified.result).toMatchObject({ status: "qualified", qualified: 8, required: 8 });

    await unlink(join(fixture.resultsPath, "safari-minimum-16-4.json"));
    const unqualified = check(fixture);
    expect(unqualified.process.status).toBe(2);
    expect(unqualified.result.status).toBe("unqualified");
    expect(unqualified.result.details).toContainEqual({
      target: "safari-minimum-16-4",
      code: "evidence_missing",
    });

    await writeFile(
      join(fixture.resultsPath, "safari-minimum-16-4.json"),
      `${JSON.stringify(evidenceFor(requiredTarget("safari-minimum-16-4"), fixture, "fail"), null, 2)}\n`,
      "utf8",
    );
    const failed = check(fixture);
    expect(failed.process.status).toBe(1);
    expect(failed.result.status).toBe("failed");
    expect(failed.result.details).toContainEqual({
      target: "safari-minimum-16-4",
      code: "conformance_failed",
    });

    await writeFile(
      join(fixture.resultsPath, "safari-minimum-16-4.json"),
      `${JSON.stringify(evidenceFor(requiredTarget("safari-minimum-16-4"), fixture), null, 2)}\n`,
      "utf8",
    );
    await writeFile(join(fixture.root, "runtime.js"), "new runtime bytes", "utf8");
    const stale = check(fixture);
    expect(stale.process.status).toBe(2);
    expect(stale.result.status).toBe("unqualified");
    expect(stale.result.details.some(({ code }) => code === "runtime_stale")).toBe(true);
  });

  it("rejects WebKit, Chromium, user-agent, and simulated evidence claims", async () => {
    const fixture = await evidenceFixture();
    await writeCompleteEvidence(fixture);
    const chrome = evidenceFor(requiredTarget("chrome-minimum-111"), fixture);
    await writeFile(
      join(fixture.resultsPath, "chrome-minimum-111.json"),
      `${JSON.stringify({ ...chrome, browserProduct: "chromium" })}\n`,
      "utf8",
    );
    let rejected = check(fixture);
    expect(rejected.process.status).toBe(1);
    expect(rejected.result.status).toBe("failed");

    await writeFile(
      join(fixture.resultsPath, "chrome-minimum-111.json"),
      `${JSON.stringify({ ...chrome, provider: "simulated user-agent claim" })}\n`,
      "utf8",
    );
    rejected = check(fixture);
    expect(rejected.process.status).toBe(1);
    expect(rejected.result.details).toContainEqual({
      target: "chrome-minimum-111",
      code: "evidence_provider_invalid",
    });

    await writeFile(
      join(fixture.resultsPath, "chrome-minimum-111.json"),
      `${JSON.stringify({ ...chrome, provider: "                  " })}\n`,
      "utf8",
    );
    rejected = check(fixture);
    expect(rejected.process.status).toBe(1);
    expect(rejected.result.details).toContainEqual({
      target: "chrome-minimum-111",
      code: "evidence_provider_invalid",
    });
  });

  it("writes evidence only after every authenticated current-artifact case receipt", async () => {
    const root = await mkdtemp(join(tmpdir(), "suprnova-live-adapter-"));
    temporaryDirectories.push(root);
    const results = join(root, "results");
    const adapter = join(root, "adapter.mjs");
    await writeFile(
      adapter,
      `export async function runCompatibility(input) {
        return {
          attestation: "urn:suprnova-live:provider-attestation:verified",
          browserProduct: input.target.browserProduct,
          browserVersion: input.target.version,
          cases: input.requiredCases.map((id) => ({ id, result: "pass", ...input.challenge })),
          identitySource: "webdriver_capabilities",
          operatingSystem: "Provider OS 2026.08",
          provider: "Actual Browser Qualification Lab",
          result: "pass",
          testRun: input.challenge,
        };
      }\n`,
      "utf8",
    );
    const execution = spawnSync(
      process.execPath,
      [RUNNER, "--target", "chrome-minimum-111", "--adapter", adapter, "--results", results],
      {
        encoding: "utf8",
        env: { ...process.env, BROWSER_PROVIDER_TOKEN: "SECRET_SENTINEL" },
        timeout: 30_000,
      },
    );
    expect(execution.status).toBe(0);
    expect(execution.stdout).toContain("chrome-minimum-111");
    expect(`${execution.stdout}${execution.stderr}`).not.toContain("SECRET_SENTINEL");
    const evidence = await readFile(join(results, "chrome-minimum-111.json"), "utf8");
    expect(evidence).not.toContain("SECRET_SENTINEL");
    expect(Object.keys(JSON.parse(evidence) as object).sort()).toEqual([
      "attestation",
      "browserProduct",
      "browserVersion",
      "executedAt",
      "fixtureManifestSha256",
      "operatingSystem",
      "provider",
      "result",
      "runtimeSha256",
      "schemaVersion",
    ]);

    await writeFile(
      adapter,
      `export async function runCompatibility(input) {
        return {
          attestation: "urn:suprnova-live:provider-attestation:incomplete",
          browserProduct: input.target.browserProduct,
          browserVersion: input.target.version,
          cases: input.requiredCases.slice(1).map((id) => ({ id, result: "pass", ...input.challenge })),
          identitySource: "webdriver_capabilities",
          operatingSystem: "Provider OS 2026.08",
          provider: "Actual Browser Qualification Lab",
          result: "pass",
          testRun: input.challenge,
        };
      }\n`,
      "utf8",
    );
    await unlink(join(results, "chrome-minimum-111.json"));
    const incomplete = spawnSync(
      process.execPath,
      [RUNNER, "--target", "chrome-minimum-111", "--adapter", adapter, "--results", results],
      { encoding: "utf8", timeout: 30_000 },
    );
    expect(incomplete.status).toBe(1);
    await expect(readFile(join(results, "chrome-minimum-111.json"), "utf8")).rejects.toThrow(
      /ENOENT/u,
    );
  }, 30_000);
});
