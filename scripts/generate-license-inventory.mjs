#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const inventoryPath = resolve(repositoryRoot, "THIRD_PARTY_LICENSES.md");
const internalCargoPackages = new Set([
  "suprnova-live",
  "suprnova-live-fuzz",
  "suprnova-live-macro-fixture",
  "suprnova-live-macros",
  "suprnova-live-test-support",
]);

function markdown(value) {
  return value.replaceAll("|", "\\|").replaceAll("\n", " ");
}

function cargoPackagesFrom(directory) {
  const result = spawnSync(
    "rtk",
    ["cargo", "metadata", "--locked", "--format-version", "1"],
    {
      cwd: directory,
      encoding: "utf8",
    },
  );

  if (result.status !== 0) {
    process.stderr.write(result.stderr);
    throw new Error(
      "cargo metadata failed while generating the license inventory",
    );
  }

  const metadata = JSON.parse(result.stdout);
  return metadata.packages
    .filter(
      (dependency) => !internalCargoPackages.has(dependency.name),
    )
    .map((dependency) => ({
      ecosystem: "Cargo",
      name: dependency.name,
      version: dependency.version,
      license: dependency.license,
      source: dependency.source ?? "workspace/path",
    }));
}

function cargoPackages() {
  const packages = [
    ...cargoPackagesFrom(repositoryRoot),
    ...cargoPackagesFrom(resolve(repositoryRoot, "fuzz")),
  ];
  return [
    ...new Map(
      packages.map((dependency) => [
        [dependency.name, dependency.version, dependency.source].join("\0"),
        dependency,
      ]),
    ).values(),
  ];
}

function npmPackageName(packagePath, dependency) {
  if (typeof dependency.name === "string") {
    return dependency.name;
  }

  const marker = "node_modules/";
  const markerIndex = packagePath.lastIndexOf(marker);
  if (markerIndex === -1) {
    throw new Error(`cannot determine npm package name for ${packagePath}`);
  }

  return packagePath.slice(markerIndex + marker.length);
}

function npmPackages() {
  const lockPath = resolve(repositoryRoot, "browser/package-lock.json");
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));

  return Object.entries(lock.packages)
    .filter(([packagePath]) => packagePath.length > 0)
    .map(([packagePath, dependency]) => ({
      ecosystem: "npm",
      name: npmPackageName(packagePath, dependency),
      version: dependency.version,
      license: dependency.license,
      source: dependency.resolved ?? "npm lockfile",
    }));
}

function requireField(dependency, field) {
  const value = dependency[field];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(
      `${dependency.ecosystem} package ${dependency.name}@${dependency.version} has no ${field}`,
    );
  }
  return value;
}

function renderInventory() {
  const dependencies = [...cargoPackages(), ...npmPackages()].sort(
    (left, right) =>
      [left.ecosystem, left.name, left.version]
        .join("\0")
        .localeCompare(
          [right.ecosystem, right.name, right.version].join("\0"),
          "en",
        ),
  );

  const rows = dependencies.map((dependency) => {
    const ecosystem = requireField(dependency, "ecosystem");
    const name = requireField(dependency, "name");
    const version = requireField(dependency, "version");
    const license = requireField(dependency, "license");
    const source = requireField(dependency, "source");
    return `| ${markdown(ecosystem)} | ${markdown(name)} | ${markdown(version)} | ${markdown(license)} | ${markdown(source)} |`;
  });

  return `# Third-party licenses

Suprnova Live is licensed under MIT. This generated inventory covers every
resolved third-party package in the checked Cargo and npm lockfiles. Regenerate
it with \`rtk node scripts/generate-license-inventory.mjs\`; the unattended gate
uses \`--check\` to reject lockfile or license drift.

| Ecosystem | Package | Version | License | Locked source |
|---|---|---:|---|---|
${rows.join("\n")}
`;
}

const expected = renderInventory();
if (process.argv.includes("--check")) {
  const observed = readFileSync(inventoryPath, "utf8");
  if (observed !== expected) {
    process.stderr.write(
      "THIRD_PARTY_LICENSES.md is stale; run rtk node scripts/generate-license-inventory.mjs\n",
    );
    process.exitCode = 1;
  }
} else {
  writeFileSync(inventoryPath, expected);
}
