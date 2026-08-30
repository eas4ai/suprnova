#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { arch, platform, release } from "node:os";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const baselinePath = resolve(
  repositoryRoot,
  "benchmarks/expansion-budget-v1.json",
);
const localResultPath = resolve(
  repositoryRoot,
  "benchmarks/local/expansion-budget-v1.json",
);
const fixtureCounts = [1, 10, 100];
const maxGrowth = 12;
const tokenPattern = /[A-Za-z_][A-Za-z0-9_]*|::|->|=>|[^\s]/gu;

function runRtk(arguments_, options = {}) {
  const result = spawnSync("rtk", arguments_, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    ...options,
  });
  if (result.status !== 0) {
    process.stderr.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    throw new Error(`rtk ${arguments_.join(" ")} failed`);
  }
  return (result.stdout ?? "").trim();
}

function fixturePaths(componentCount) {
  const directory = resolve(
    repositoryRoot,
    `tests/fixtures/compile/${componentCount}-component`,
  );
  return {
    directory,
    manifest: resolve(directory, "Cargo.toml"),
    source: resolve(directory, "src/lib.rs"),
    target: resolve(
      repositoryRoot,
      `benchmarks/local/expansion-target-${componentCount}`,
    ),
  };
}

function fixtureDigest(paths) {
  return createHash("sha256")
    .update(readFileSync(paths.manifest))
    .update("\0")
    .update(readFileSync(paths.source))
    .digest("hex");
}

function measureFixture(componentCount) {
  const paths = fixturePaths(componentCount);
  rmSync(paths.target, { force: true, recursive: true });
  const environment = {
    ...process.env,
    CARGO_INCREMENTAL: "0",
    CARGO_TARGET_DIR: paths.target,
  };

  const checkStarted = performance.now();
  runRtk(
    ["cargo", "check", "--locked", "--manifest-path", paths.manifest],
    { env: environment },
  );
  const cargoCheckMilliseconds = Math.round(performance.now() - checkStarted);

  const expanded = runRtk(
    [
      "cargo",
      "+nightly",
      "rustc",
      "--locked",
      "--manifest-path",
      paths.manifest,
      "--lib",
      "--",
      "-Zunpretty=expanded",
    ],
    { env: environment },
  );
  const expandedTokens = expanded.match(tokenPattern)?.length ?? 0;
  if (expandedTokens === 0) {
    throw new Error(`${componentCount}-component expansion produced no tokens`);
  }

  return {
    component_count: componentCount,
    expanded_tokens: expandedTokens,
    expanded_bytes: Buffer.byteLength(expanded, "utf8"),
    cargo_check_milliseconds: cargoCheckMilliseconds,
    fixture_sha256: fixtureDigest(paths),
  };
}

function assertLinear(fixtures) {
  for (const metric of [
    "expanded_tokens",
    "expanded_bytes",
    "cargo_check_milliseconds",
  ]) {
    for (let index = 1; index < fixtures.length; index += 1) {
      const ratio = fixtures[index][metric] / fixtures[index - 1][metric];
      if (ratio > maxGrowth) {
        throw new Error(
          `${metric} grew ${ratio.toFixed(2)}x from ${fixtures[index - 1].component_count} to ${fixtures[index].component_count} components`,
        );
      }
    }
  }
}

function assertBaseline(observed, baseline) {
  if (baseline.schema_version !== 1 || baseline.workload !== "component-expansion") {
    throw new Error("checked expansion baseline has an unsupported schema");
  }
  for (const fixture of observed.fixtures) {
    const expected = baseline.fixtures.find(
      (candidate) => candidate.component_count === fixture.component_count,
    );
    if (!expected || expected.fixture_sha256 !== fixture.fixture_sha256) {
      throw new Error(
        `${fixture.component_count}-component fixture drifted; regenerate and review the baseline`,
      );
    }
    for (const metric of ["expanded_tokens", "expanded_bytes"]) {
      if (fixture[metric] > expected[metric] * 1.1) {
        throw new Error(
          `${fixture.component_count}-component ${metric} regressed by more than 10%`,
        );
      }
    }
    if (
      fixture.cargo_check_milliseconds >
      expected.cargo_check_milliseconds * 2.5 + 2_000
    ) {
      throw new Error(
        `${fixture.component_count}-component isolated cargo check regressed materially`,
      );
    }
  }
}

const fixtures = fixtureCounts.map(measureFixture);
assertLinear(fixtures);
const observed = {
  schema_version: 1,
  workload: "component-expansion",
  profile: "cargo-check",
  fixtures,
  measured_at_unix_ms: Date.now(),
  environment: {
    classification: "local_exploratory",
    operating_system: platform(),
    architecture: arch(),
    kernel: release(),
    rustc: runRtk(["proxy", "rustc", "--version"]),
    nightly_rustc: runRtk([
      "proxy",
      "rustup",
      "run",
      "nightly",
      "rustc",
      "--version",
    ]),
    cargo: runRtk(["cargo", "--version"]),
    release_qualified: false,
  },
};

const writeBaseline = process.argv.includes("--write");
const outputPath = writeBaseline ? baselinePath : localResultPath;
if (!writeBaseline) {
  assertBaseline(observed, JSON.parse(readFileSync(baselinePath, "utf8")));
}
mkdirSync(dirname(outputPath), { recursive: true });
const temporaryOutputPath = `${outputPath}.tmp`;
writeFileSync(temporaryOutputPath, `${JSON.stringify(observed, null, 2)}\n`);
renameSync(temporaryOutputPath, outputPath);

for (const fixture of fixtures) {
  process.stdout.write(
    `[expansion-budget] ${fixture.component_count} components: ${fixture.expanded_tokens} tokens, ${fixture.expanded_bytes} bytes, ${fixture.cargo_check_milliseconds} ms cargo check\n`,
  );
}
process.stdout.write(
  `[expansion-budget] ${writeBaseline ? "baseline recorded" : "baseline and linear-growth checks passed"}\n`,
);
