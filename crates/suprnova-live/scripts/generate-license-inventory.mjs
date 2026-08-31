#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  cargoDependencyClosure,
  loadLockedCargoMetadata,
  resolveGitWorkspaceRoot,
} from "./license-inventory-cargo.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const liveRoot = resolve(scriptDirectory, "..");
const workspaceRoot = resolveGitWorkspaceRoot(liveRoot);
const liveRelative = relative(workspaceRoot, liveRoot);
if (
  liveRelative.length === 0 ||
  liveRelative === ".." ||
  liveRelative.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) ||
  isAbsolute(liveRelative)
) {
  throw new Error(
    `Suprnova Live root is outside its parent workspace: ${liveRoot}`,
  );
}
const inventoryPath = resolve(liveRoot, "THIRD_PARTY_LICENSES.md");
const liveWorkspaceCargoRoots = [
  "suprnova-live",
  "suprnova-live-macro-fixture",
  "suprnova-live-macros",
  "suprnova-live-test-support",
];
const fuzzCargoRoots = ["suprnova-live-fuzz"];
const compileFixtureCargoRoots = [
  "suprnova-live-compile-1",
  "suprnova-live-compile-10",
  "suprnova-live-compile-100",
];
const internalCargoPackages = new Set([
  "suprnova-live",
  "suprnova-live-fuzz",
  "suprnova-live-macro-fixture",
  "suprnova-live-macros",
  "suprnova-live-test-support",
  "suprnova-live-compile-1",
  "suprnova-live-compile-10",
  "suprnova-live-compile-100",
]);
const npmBuildDependencies = new Set(["esbuild", "terser"]);
const npmTestDependencies = new Set([
  "@hotwired/stimulus",
  "@playwright/test",
  "axe-core",
  "fast-check",
  "vitest",
]);
const usagePriority = new Map([
  ["Development tooling", 0],
  ["Test only", 1],
  ["Production build", 2],
  ["Production runtime", 3],
]);

function markdown(value) {
  return value.replaceAll("|", "\\|").replaceAll("\n", " ");
}

function cargoPackagesFrom(directory, rootPackageNames) {
  const metadata = loadLockedCargoMetadata(directory);
  return cargoDependencyClosure(metadata, rootPackageNames)
    .filter((dependency) => !internalCargoPackages.has(dependency.name))
    .map((dependency) => ({
      ecosystem: "Cargo",
      name: dependency.name,
      version: dependency.version,
      usage: "Workspace resolved",
      license: dependency.license,
      source: dependency.source ?? "workspace/path",
    }));
}

function cargoPackages() {
  const packages = [
    ...cargoPackagesFrom(workspaceRoot, liveWorkspaceCargoRoots),
    ...cargoPackagesFrom(resolve(liveRoot, "fuzz"), fuzzCargoRoots),
    ...cargoPackagesFrom(
      resolve(liveRoot, "tests/fixtures/compile"),
      compileFixtureCargoRoots,
    ),
  ];
  const unique = new Map();
  for (const dependency of packages) {
    const key = [dependency.name, dependency.version, dependency.source].join(
      "\0",
    );
    const current = unique.get(key);
    if (
      current === undefined ||
      usagePriority.get(current.usage) < usagePriority.get(dependency.usage)
    ) {
      unique.set(key, dependency);
    }
  }
  return [...unique.values()];
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

function npmDependencyPath(packages, packagePath, name) {
  let current = packagePath;
  while (true) {
    const candidate = current
      ? `${current}/node_modules/${name}`
      : `node_modules/${name}`;
    if (Object.hasOwn(packages, candidate)) return candidate;
    if (current === "") return null;
    const ancestor = current.lastIndexOf("/node_modules/");
    current = ancestor === -1 ? "" : current.slice(0, ancestor);
  }
}

function npmDependencyNames(dependency) {
  return Object.keys({
    ...(dependency.dependencies ?? {}),
    ...(dependency.optionalDependencies ?? {}),
  });
}

function npmUsageByPath(lock) {
  const packages = lock.packages;
  const root = packages[""];
  if (typeof root !== "object" || root === null || Array.isArray(root)) {
    throw new Error("browser package-lock.json has no root package");
  }

  const usages = new Map();
  const pending = [];
  const enqueue = (packagePath, usage) => {
    const current = usages.get(packagePath);
    if (
      current !== undefined &&
      usagePriority.get(current) >= usagePriority.get(usage)
    ) {
      return;
    }
    usages.set(packagePath, usage);
    pending.push({ packagePath, usage });
  };

  for (const name of Object.keys(root.dependencies ?? {})) {
    const packagePath = npmDependencyPath(packages, "", name);
    if (packagePath === null)
      throw new Error(`npm dependency ${name} is not locked`);
    enqueue(packagePath, "Production runtime");
  }
  for (const name of Object.keys(root.devDependencies ?? {})) {
    const packagePath = npmDependencyPath(packages, "", name);
    if (packagePath === null)
      throw new Error(`npm dependency ${name} is not locked`);
    const usage = npmBuildDependencies.has(name)
      ? "Production build"
      : npmTestDependencies.has(name)
        ? "Test only"
        : "Development tooling";
    enqueue(packagePath, usage);
  }

  while (pending.length > 0) {
    const next = pending.shift();
    const dependency = packages[next.packagePath];
    for (const name of npmDependencyNames(dependency)) {
      const packagePath = npmDependencyPath(packages, next.packagePath, name);
      if (packagePath === null) {
        throw new Error(
          `npm dependency ${name} required by ${next.packagePath} is not locked`,
        );
      }
      enqueue(packagePath, next.usage);
    }
  }

  return usages;
}

function npmPackages() {
  const lockPath = resolve(liveRoot, "browser/package-lock.json");
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));
  const usages = npmUsageByPath(lock);

  const packages = Object.entries(lock.packages)
    .filter(([packagePath]) => packagePath.length > 0)
    .map(([packagePath, dependency]) => {
      const usage = usages.get(packagePath);
      if (usage === undefined) {
        throw new Error(
          `npm package ${packagePath} is unreachable from the root package`,
        );
      }
      return {
        ecosystem: "npm",
        name: npmPackageName(packagePath, dependency),
        version: dependency.version,
        usage,
        license: dependency.license,
        source: dependency.resolved ?? "npm lockfile",
      };
    });

  return [
    ...new Map(
      packages.map((dependency) => [
        [dependency.name, dependency.version, dependency.source].join("\0"),
        dependency,
      ]),
    ).values(),
  ];
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
    const usage = requireField(dependency, "usage");
    const license = requireField(dependency, "license");
    const source = requireField(dependency, "source");
    return `| ${markdown(ecosystem)} | ${markdown(name)} | ${markdown(version)} | ${markdown(usage)} | ${markdown(license)} | ${markdown(source)} |`;
  });

  return `# Third-party licenses

Suprnova Live is licensed under MIT. For Cargo, this generated inventory covers
the conservative dependency closure reachable from the four Live package roots
in the shared Suprnova resolution, plus the separately resolved fuzz and compile
fixture roots. Unrelated parent-workspace roots and their unreachable
dependencies are excluded. Regenerate it with
\`rtk node scripts/generate-license-inventory.mjs\`; the unattended gate uses
\`--check\` to reject lockfile or license drift.

Cargo feature unification is shared-workspace-wide, so this conservative closure
can include optional dependency edges enabled elsewhere in the workspace. A
\`Workspace resolved\` row records reachability in those resolved graphs; it does
not claim exact \`cargo tree\` use by every Live build.

For npm, usage is derived transitively from the exact root dependency graph.
Production runtime takes precedence over production build, test-only, and
development-tooling reachability. The production asset manifest and JavaScript
banner separately retain Idiomorph's name, version, and 0BSD license metadata.

| Ecosystem | Package | Version | Usage | License | Locked source |
|---|---|---:|---|---|---|
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
